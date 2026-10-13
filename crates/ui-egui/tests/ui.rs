//! Drives the interface headlessly against a backend that only records what it is sent.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use craft_proton_sync_core::protocol::{Activity, Backend, Command, Phase, Snapshot, Update};
use craft_proton_sync_core::settings::Settings;
use craft_proton_sync_ui_egui::{Host, SyncUi, theme};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

struct Fake {
    snapshot: Snapshot,
    sent: RefCell<Vec<Command>>,
}

impl Backend for Fake {
    fn snapshot(&self) -> Snapshot {
        self.snapshot.clone()
    }

    fn send(&self, command: Command) {
        self.sent.borrow_mut().push(command);
    }
}

#[derive(Default)]
struct Desk {
    opened: Vec<PathBuf>,
    pages: Vec<String>,
    copied: Vec<String>,
    saved: Vec<Settings>,
    login: bool,
}

impl Host for Desk {
    fn open_folder(&mut self, path: &Path) {
        self.opened.push(path.to_path_buf());
    }

    fn open_sign_in_page(&mut self, url: &str) {
        self.pages.push(url.to_string());
    }

    fn copy_text(&mut self, text: &str) {
        self.copied.push(text.to_string());
    }

    fn login_item(&self) -> Option<bool> {
        Some(self.login)
    }

    fn set_login_item(&mut self, on: bool) -> Result<(), String> {
        self.login = on;
        Ok(())
    }

    fn has_tray(&self) -> bool {
        true
    }

    fn settings_changed(&mut self, settings: &Settings) {
        self.saved.push(settings.clone());
    }

    fn now(&self) -> u64 {
        1_000_000
    }
}

struct State {
    ui: SyncUi,
    backend: Fake,
    desk: Desk,
}

fn harness(phase: Phase, change: impl FnOnce(&mut Snapshot)) -> Harness<'static, State> {
    harness_sized(phase, 900.0, change)
}

fn harness_sized(phase: Phase, height: f32, change: impl FnOnce(&mut Snapshot)) -> Harness<'static, State> {
    let mut snapshot = Snapshot::new(PathBuf::from("Documents").join("ArtCraft"));
    snapshot.phase = phase;
    change(&mut snapshot);
    let state = State {
        ui: SyncUi::new(Settings::default()),
        backend: Fake {
            snapshot,
            sent: RefCell::new(Vec::new()),
        },
        desk: Desk::default(),
    };
    let mut harness = Harness::builder().with_size(egui::vec2(480.0, height)).build_ui_state(
        |ui, state: &mut State| {
            state.ui.ui(ui, &state.backend, &mut state.desk);
        },
        state,
    );
    theme::install(&harness.ctx);
    harness.run_steps(3);
    harness
}

fn signed_in(s: &mut Snapshot) {
    s.account = Some("artist@proton.me".into());
    s.last_sync = Some(1_000_000 - 180);
}

#[test]
fn every_phase_draws() {
    for phase in [
        Phase::Starting,
        Phase::InstallingEngine {
            received: 5 << 20,
            total: Some(25 << 20),
        },
        Phase::InstallingEngine {
            received: 5 << 20,
            total: None,
        },
        Phase::EngineFailed,
        Phase::SignedOut,
        Phase::SigningIn,
        Phase::Idle,
        Phase::Syncing,
        Phase::Paused,
        Phase::SyncFailed,
    ] {
        harness(phase, |s| s.error = Some("a reason".into()));
    }
}

#[test]
fn signing_in_starts_on_protons_page() {
    let mut h = harness(Phase::SignedOut, |_| {});
    assert!(h.query_by_label("CANCEL").is_none());
    h.get_by_label("SIGN IN WITH PROTON").click();
    h.run_steps(3);
    assert_eq!(*h.state().backend.sent.borrow(), [Command::SignIn]);
}

#[test]
fn a_sign_in_under_way_can_be_reopened_copied_or_cancelled() {
    let url = "https://account.proton.me/desktop/login?app=drive#payload=abc";
    let mut h = harness(Phase::SigningIn, |s| s.sign_in_url = Some(url.into()));
    assert!(h.query_by_label("SIGN IN WITH PROTON").is_none());
    h.get_by_label("OPEN THE PAGE AGAIN").click();
    h.run_steps(3);
    h.get_by_label("COPY THE LINK").click();
    h.run_steps(3);
    h.get_by_label("CANCEL").click();
    h.run_steps(3);
    let state = h.state();
    assert_eq!(state.desk.pages, [url]);
    assert_eq!(state.desk.copied, [url]);
    assert_eq!(*state.backend.sent.borrow(), [Command::CancelSignIn]);
}

#[test]
fn a_failed_sign_in_says_why() {
    let h = harness(Phase::SignedOut, |s| s.sign_in_error = Some("Incorrect email or password.".into()));
    h.get_by_label("Incorrect email or password.");
}

#[test]
fn the_synced_view_shows_the_account_and_the_last_sync() {
    let h = harness(Phase::Idle, |s| {
        signed_in(s);
        s.activity = vec![Activity {
            when: 1_000_000 - 30,
            action: "Uploaded",
            path: "PhotoCraft/beach.pcraft".into(),
        }];
    });
    h.get_by_label("artist@proton.me");
    h.get_by_label("Last synced 3 minutes ago");
    h.get_by_label("PhotoCraft/beach.pcraft");
    h.get_by_label("UP TO DATE");
}

#[test]
fn the_buttons_send_their_commands() {
    let mut h = harness(Phase::Idle, signed_in);
    h.get_by_label("SYNC NOW").click();
    h.run_steps(3);
    h.get_by_label("PAUSE").click();
    h.run_steps(3);
    h.get_by_label("OPEN FOLDER").click();
    h.run_steps(3);
    h.get_by_label("SIGN OUT").click();
    h.run_steps(3);
    let state = h.state();
    assert_eq!(
        *state.backend.sent.borrow(),
        [Command::SyncNow, Command::SetPaused(true), Command::SignOut]
    );
    assert_eq!(state.desk.opened, [PathBuf::from("Documents").join("ArtCraft")]);
    assert!(state.desk.saved.last().is_some_and(|s| s.paused), "pausing is remembered");
}

#[test]
fn a_failed_sync_shows_its_reason() {
    let h = harness(Phase::SyncFailed, |s| {
        signed_in(s);
        s.error = Some("Couldn't reach Proton Drive. It will be tried again.".into());
    });
    h.get_by_label("Couldn't reach Proton Drive. It will be tried again.");
    h.get_by_label("SYNC FAILED");
}

#[test]
fn a_failed_setup_can_be_tried_again() {
    let mut h = harness(Phase::EngineFailed, |s| {
        s.error = Some("couldn't download the Proton Drive CLI".into())
    });
    h.get_by_label("couldn't download the Proton Drive CLI");
    h.get_by_label("TRY AGAIN").click();
    h.run_steps(3);
    assert_eq!(*h.state().backend.sent.borrow(), [Command::RetryInstall]);
}

const VERSION: &str = env!("CARGO_PKG_VERSION");

#[test]
fn a_newer_version_can_be_installed_with_one_click() {
    let mut h = harness_sized(Phase::Idle, 1200.0, |s| {
        signed_in(s);
        s.update = Update::Available { version: "9.9.9".into() };
    });
    h.get_by_label(&format!("Version 9.9.9 is out (you have {VERSION})."));
    h.get_by_label("UPDATE NOW").click();
    h.run_steps(3);
    assert_eq!(*h.state().backend.sent.borrow(), [Command::InstallUpdate]);
}

#[test]
fn updates_are_offered_before_signing_in_too() {
    let h = harness(Phase::SignedOut, |s| s.update = Update::Available { version: "9.9.9".into() });
    h.get_by_label("UPDATE NOW");
}

#[test]
fn automatic_updates_are_off_until_turned_on() {
    let mut h = harness_sized(Phase::Idle, 1200.0, |s| {
        signed_in(s);
        s.update = Update::UpToDate;
    });
    assert!(!h.state().ui.settings.auto_update);
    h.get_by_label(&format!("Version {VERSION} is the latest."));
    h.get_by_label("Install new versions automatically").click();
    h.run_steps(3);
    let state = h.state();
    assert_eq!(*state.backend.sent.borrow(), [Command::SetAutoUpdate(true)]);
    assert!(state.desk.saved.last().is_some_and(|s| s.auto_update), "the choice is remembered");
}

#[test]
fn checking_again_and_failures_are_shown() {
    let mut h = harness_sized(Phase::Idle, 1200.0, |s| {
        signed_in(s);
        s.update = Update::Failed("couldn't check for updates (offline)".into());
    });
    h.get_by_label("couldn't check for updates (offline)");
    h.get_by_label("TRY AGAIN").click();
    h.run_steps(3);
    assert_eq!(*h.state().backend.sent.borrow(), [Command::CheckForUpdate]);
}

#[test]
fn every_update_state_draws() {
    for update in [
        Update::Unsupported,
        Update::Unchecked,
        Update::Checking,
        Update::UpToDate,
        Update::Installing {
            version: "9.9.9".into(),
            received: 1 << 20,
            total: Some(7 << 20),
        },
        Update::Installing {
            version: "9.9.9".into(),
            received: 0,
            total: None,
        },
        Update::Restarting,
    ] {
        let h = harness_sized(Phase::Idle, 1200.0, |s| {
            signed_in(s);
            s.update = update.clone();
        });
        assert!(h.query_by_label("UPDATE NOW").is_none(), "{update:?}");
    }
}
