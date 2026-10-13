//! The sync engine: one thread that installs the Proton Drive CLI, signs in, watches the folder
//! and syncs.
//!
//! A sync runs a few seconds after the last change in the folder (saves come in bursts), and on a
//! timer for changes made on other devices.

use std::path::PathBuf;
use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use craft_proton_sync_core::layout;
use craft_proton_sync_core::protocol::{Activity, Backend, Command, Phase, Snapshot, Update};
use craft_proton_sync_core::settings::{MAX_POLL_MINUTES, MIN_POLL_MINUTES, Settings};
use craft_proton_sync_core::update::Release;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};

use crate::account::{self, SignIn};
use crate::drive::{Cli, Session};
use crate::paths::Paths;
use crate::sync::{self, SyncError, Syncer};
use crate::{install, lock, open, unix_now, update};

/// How long the folder must be quiet before a change is synced.
const DEBOUNCE: Duration = Duration::from_secs(3);
const IDLE: Duration = Duration::from_secs(3600);
const MAX_ACTIVITY: usize = 50;
/// How often the project's releases are looked at for a newer version.
const UPDATE_CHECK: Duration = Duration::from_secs(24 * 3600);
/// How soon a check that failed (offline, say) is tried again.
const UPDATE_RETRY: Duration = Duration::from_secs(3600);
const VERSION: &str = env!("CARGO_PKG_VERSION");

enum Msg {
    Command(Command),
    LocalChange,
    Shutdown,
}

struct Shared {
    snapshot: Mutex<Snapshot>,
    /// The CLI running a sync or a sign-in, for [`Engine::shutdown`] and a cancelled sign-in to stop.
    child: Mutex<Option<Child>>,
    stopping: AtomicBool,
    wake: Box<dyn Fn() + Send + Sync>,
}

impl Shared {
    fn update(&self, change: impl FnOnce(&mut Snapshot)) {
        change(&mut lock(&self.snapshot));
        (self.wake)();
    }

    fn stop_child(&self) {
        if let Some(mut child) = lock(&self.child).take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

pub struct Engine {
    tx: Sender<Msg>,
    shared: Arc<Shared>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

impl Engine {
    /// Starts the engine. `wake` is called, from the engine's thread, whenever the snapshot changes.
    pub fn start(paths: Paths, settings: &Settings, local_root: PathBuf, wake: impl Fn() + Send + Sync + 'static) -> Self {
        let shared = Arc::new(Shared {
            snapshot: Mutex::new(Snapshot::new(local_root.clone())),
            child: Mutex::new(None),
            stopping: AtomicBool::new(false),
            wake: Box::new(wake),
        });
        let (tx, rx) = channel();
        let worker = Worker {
            paths,
            local_root,
            shared: shared.clone(),
            rx,
            tx: tx.clone(),
            poll: poll_interval(settings.poll_minutes),
            paused: settings.paused,
            cli: None,
            account: None,
            watcher: None,
            dirty: None,
            next_poll: Instant::now(),
            error: None,
            auto_update: settings.auto_update,
            next_update_check: None,
            available: None,
            restarting: false,
        };
        let handle = std::thread::Builder::new().name("sync-engine".into()).spawn(move || worker.run());
        let worker = match handle {
            Ok(handle) => Some(handle),
            Err(e) => {
                shared.update(|s| {
                    s.phase = Phase::EngineFailed;
                    s.error = Some(format!("couldn't start the sync engine ({e})"));
                });
                None
            }
        };
        Self {
            tx,
            shared,
            worker: Mutex::new(worker),
        }
    }

    /// Stops a running sync and the engine's thread. A sync cut short is picked up by the next one.
    pub fn shutdown(&self) {
        self.shared.stopping.store(true, Ordering::SeqCst);
        self.shared.stop_child();
        let _ = self.tx.send(Msg::Shutdown);
        if let Some(worker) = lock(&self.worker).take() {
            let _ = worker.join();
        }
    }
}

impl Backend for Engine {
    fn snapshot(&self) -> Snapshot {
        lock(&self.shared.snapshot).clone()
    }

    fn send(&self, command: Command) {
        // The engine's thread is busy waiting on the sign-in, so cancelling can't queue behind it.
        if command == Command::CancelSignIn {
            if lock(&self.shared.snapshot).phase == Phase::SigningIn {
                self.shared.stop_child();
            }
            return;
        }
        let _ = self.tx.send(Msg::Command(command));
    }
}

fn poll_interval(minutes: u32) -> Duration {
    Duration::from_secs(u64::from(minutes.clamp(MIN_POLL_MINUTES, MAX_POLL_MINUTES)) * 60)
}

struct Worker {
    paths: Paths,
    local_root: PathBuf,
    shared: Arc<Shared>,
    rx: Receiver<Msg>,
    /// For the folder watcher.
    tx: Sender<Msg>,
    poll: Duration,
    paused: bool,
    cli: Option<Cli>,
    account: Option<String>,
    watcher: Option<RecommendedWatcher>,
    /// When the folder last changed, if that hasn't been synced yet.
    dirty: Option<Instant>,
    next_poll: Instant,
    /// Why the last sync failed.
    error: Option<String>,
    auto_update: bool,
    /// When the releases are next looked at; `None` in builds that don't update themselves.
    next_update_check: Option<Instant>,
    /// A newer release that was found.
    available: Option<Release>,
    /// An update is in place and the app is about to start again: nothing more is begun.
    restarting: bool,
}

impl Worker {
    fn run(mut self) {
        self.install();
        if update::supported() {
            self.next_update_check = Some(Instant::now());
        } else {
            self.shared.update(|s| s.update = Update::Unsupported);
        }
        loop {
            match self.rx.recv_timeout(self.wait()) {
                Ok(Msg::LocalChange) => self.dirty = Some(Instant::now()),
                Ok(Msg::Command(command)) => self.handle(command),
                Ok(Msg::Shutdown) | Err(RecvTimeoutError::Disconnected) => return,
                Err(RecvTimeoutError::Timeout) => {}
            }
            if self.shared.stopping.load(Ordering::SeqCst) {
                return;
            }
            if self.next_update_check.is_some_and(|at| Instant::now() >= at) {
                self.check_update();
            }
            if self.due() {
                self.sync();
            }
        }
    }

    fn can_sync(&self) -> bool {
        self.cli.is_some() && self.account.is_some() && !self.paused && !self.restarting
    }

    fn debounce_left(&self) -> Option<Duration> {
        self.dirty.map(|since| DEBOUNCE.saturating_sub(since.elapsed()))
    }

    fn due(&self) -> bool {
        self.can_sync() && (self.debounce_left().is_some_and(|left| left.is_zero()) || Instant::now() >= self.next_poll)
    }

    /// How long to sleep before the next sync could be due.
    fn wait(&self) -> Duration {
        let update = self
            .next_update_check
            .map_or(IDLE, |at| at.saturating_duration_since(Instant::now()));
        if !self.can_sync() {
            return IDLE.min(update);
        }
        let poll = self.next_poll.saturating_duration_since(Instant::now());
        self.debounce_left().map_or(poll, |left| left.min(poll)).min(update)
    }

    /// Sets the phase from what the worker knows, once it is at rest.
    fn settle(&self) {
        let phase = if self.cli.is_none() {
            Phase::EngineFailed
        } else if self.account.is_none() {
            Phase::SignedOut
        } else if self.paused {
            Phase::Paused
        } else if self.error.is_some() {
            Phase::SyncFailed
        } else {
            Phase::Idle
        };
        let (account, error) = (self.account.clone(), self.error.clone());
        self.shared.update(|s| {
            // A failed install keeps its own message.
            if phase != Phase::EngineFailed {
                s.error = error;
            }
            s.phase = phase;
            s.account = account;
            s.sign_in_url = None;
        });
    }

    fn install(&mut self) {
        self.shared.update(|s| {
            s.phase = Phase::InstallingEngine { received: 0, total: None };
            s.error = None;
        });
        let shared = self.shared.clone();
        let mut last_percent = u64::MAX;
        let mut progress = |received: u64, total: Option<u64>| {
            // The UI is woken once per percent, not once per chunk.
            let percent = total
                .filter(|t| *t > 0)
                .map_or(received / (1024 * 1024), |t| received.saturating_mul(100) / t);
            if percent != last_percent {
                last_percent = percent;
                shared.update(|s| s.phase = Phase::InstallingEngine { received, total });
            }
        };
        match install::ensure(&self.paths, &mut progress) {
            Ok(cli) => {
                self.shared.update(|s| s.phase = Phase::Starting);
                self.account = account::current(&cli);
                self.cli = Some(cli);
                if self.account.is_some() {
                    self.watch();
                    self.next_poll = Instant::now();
                }
            }
            Err(e) => {
                log::error!("{e}");
                self.shared.update(|s| s.error = Some(e));
            }
        }
        self.settle();
    }

    fn syncer(&self) -> Syncer {
        Syncer {
            local: self.local_root.clone(),
            account: self.account.clone().unwrap_or_default(),
            state: self.paths.sync_state(),
            staging: self.paths.staging(),
            trash_local: sync::recycle,
        }
    }

    /// Starts watching the synced folder. Without a watcher the timer still syncs.
    fn watch(&mut self) {
        if self.watcher.is_some() {
            return;
        }
        if let Err(e) = std::fs::create_dir_all(&self.local_root) {
            log::warn!("couldn't create {} ({e})", self.local_root.display());
            return;
        }
        let tx = self.tx.clone();
        let watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
            let Ok(event) = event else { return };
            if !matches!(event.kind, EventKind::Access(_)) && event.paths.iter().any(|p| !layout::is_transient(p)) {
                let _ = tx.send(Msg::LocalChange);
            }
        })
        .and_then(|mut w| w.watch(&self.local_root, RecursiveMode::Recursive).map(|()| w));
        match watcher {
            Ok(w) => self.watcher = Some(w),
            Err(e) => log::warn!("couldn't watch {} ({e}): syncing on the timer only", self.local_root.display()),
        }
    }

    fn signed_out(&mut self) {
        self.account = None;
        self.watcher = None;
        self.dirty = None;
        self.error = None;
    }

    fn handle(&mut self, command: Command) {
        match command {
            Command::RetryInstall => {
                if self.cli.is_none() {
                    self.install();
                }
                return;
            }
            Command::SetPollMinutes(minutes) => {
                self.poll = poll_interval(minutes);
                self.next_poll = self.next_poll.min(Instant::now() + self.poll);
                return;
            }
            // Handled where it is sent: this thread is waiting on the sign-in it cancels.
            Command::CancelSignIn => return,
            Command::CheckForUpdate => {
                if update::supported() && !self.restarting {
                    self.check_update();
                }
                return;
            }
            Command::InstallUpdate => {
                self.install_update();
                return;
            }
            Command::SetAutoUpdate(on) => {
                self.auto_update = on;
                if on {
                    self.install_update();
                }
                return;
            }
            Command::SetPaused(paused) => {
                self.paused = paused;
                if !paused {
                    self.next_poll = Instant::now();
                }
            }
            Command::SyncNow => self.next_poll = Instant::now(),
            Command::SignIn => self.sign_in(),
            Command::SignOut => {
                let Some(cli) = self.cli.clone() else { return };
                if let Err(e) = account::sign_out(&cli) {
                    log::warn!("{e}");
                }
                self.syncer().forget();
                self.signed_out();
                self.shared.update(|s| {
                    s.activity.clear();
                    s.last_sync = None;
                    s.sign_in_error = None;
                });
            }
        }
        self.settle();
    }

    /// Looks at the project's releases for a newer version, and installs it if the user asked
    /// for that to happen by itself.
    fn check_update(&mut self) {
        self.shared.update(|s| s.update = Update::Checking);
        match update::check(VERSION) {
            Ok(found) => {
                self.next_update_check = Some(Instant::now() + UPDATE_CHECK);
                let state = found.as_ref().map_or(Update::UpToDate, |r| Update::Available {
                    version: r.version.clone(),
                });
                self.available = found;
                self.shared.update(|s| s.update = state);
                if self.auto_update {
                    self.install_update();
                }
            }
            Err(e) => {
                log::warn!("{e}");
                self.next_update_check = Some(Instant::now() + UPDATE_RETRY);
                self.shared.update(|s| s.update = Update::Failed(e));
            }
        }
    }

    /// Installs the newer release that was found. Runs between syncs, never during one.
    fn install_update(&mut self) {
        let Some(release) = self.available.clone().filter(|_| !self.restarting) else {
            return;
        };
        let version = release.version.clone();
        log::info!("updating to version {version}");
        let shared = self.shared.clone();
        let mut last_percent = u64::MAX;
        let mut progress = |received: u64, total: Option<u64>| {
            let percent = total
                .filter(|t| *t > 0)
                .map_or(received / (1024 * 1024), |t| received.saturating_mul(100) / t);
            if percent != last_percent {
                last_percent = percent;
                shared.update(|s| {
                    s.update = Update::Installing {
                        version: version.clone(),
                        received,
                        total,
                    }
                });
            }
        };
        progress(0, None);
        match update::install(&release, &self.paths, &mut progress) {
            Ok(()) => {
                self.restarting = true;
                self.next_update_check = None;
                self.shared.update(|s| s.update = Update::Restarting);
            }
            Err(e) => {
                log::warn!("the update failed: {e}");
                self.shared.update(|s| s.update = Update::Failed(e));
            }
        }
    }

    fn sign_in(&mut self) {
        let Some(cli) = self.cli.clone() else { return };
        if self.account.is_some() {
            return;
        }
        self.shared.update(|s| {
            s.phase = Phase::SigningIn;
            s.sign_in_error = None;
            s.sign_in_url = None;
        });
        let shared = self.shared.clone();
        let result = account::sign_in(&cli, &self.shared.child, |url| {
            if let Err(e) = open::sign_in_page(url) {
                log::warn!("{e}");
            }
            shared.update(|s| s.sign_in_url = Some(url.to_string()));
        });
        match result {
            SignIn::Done => {
                log::info!("signed in to Proton Drive");
                self.account = account::current(&cli);
                self.error = None;
                if self.account.is_some() {
                    self.watch();
                    self.next_poll = Instant::now();
                } else {
                    self.shared
                        .update(|s| s.sign_in_error = Some("Signed in, but the session wasn't kept. Please try again.".into()));
                }
            }
            SignIn::Cancelled => {}
            SignIn::Failed(e) => self.shared.update(|s| s.sign_in_error = Some(e)),
        }
    }

    fn sync(&mut self) {
        let Some(cli) = self.cli.clone() else { return };
        self.dirty = None;
        self.shared.update(|s| s.phase = Phase::Syncing);
        let shared = self.shared.clone();
        let (started, mut files) = (Instant::now(), 0usize);
        let mut on_activity = |action: &'static str, path: &str| {
            files += 1;
            log::info!("{action}: {path}");
            shared.update(|s| {
                s.activity.insert(
                    0,
                    Activity {
                        when: unix_now(),
                        action,
                        path: path.to_string(),
                    },
                );
                s.activity.truncate(MAX_ACTIVITY);
            });
        };
        let result = match Session::open(&cli, &self.shared.child) {
            Ok(mut session) => self.syncer().run(&mut session, unix_now(), &mut on_activity),
            Err(crate::drive::RemoteError::SignedOut) => Err(SyncError::SessionExpired),
            Err(crate::drive::RemoteError::Stopped) => Err(SyncError::Stopped),
            Err(e) => Err(SyncError::Failed(e.to_string())),
        };
        self.next_poll = Instant::now() + self.poll;
        match result {
            Ok(()) => {
                log::info!("synced {files} file(s) in {:.1}s", started.elapsed().as_secs_f32());
                self.error = None;
                self.shared.update(|s| s.last_sync = Some(unix_now()));
            }
            Err(SyncError::Stopped) => return,
            Err(SyncError::SessionExpired) => {
                log::warn!("the Proton session ended");
                self.signed_out();
                self.shared
                    .update(|s| s.sign_in_error = Some("Your Proton session ended. Sign in again to carry on syncing.".into()));
            }
            Err(SyncError::Failed(e)) => {
                log::warn!("sync failed: {e}");
                self.error = Some(e);
            }
        }
        self.settle();
    }
}
