//! What the UI and the sync engine say to each other.
//!
//! The UI reads a [`Snapshot`] each frame and sends [`Command`]s; the engine does the work on its
//! own thread and wakes the UI when the snapshot changes.

use std::path::PathBuf;

/// What the engine is doing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    Starting,
    /// Downloading the Proton Drive CLI, the sync engine, on first run.
    InstallingEngine {
        received: u64,
        total: Option<u64>,
    },
    /// The CLI could not be downloaded or verified; [`Snapshot::error`] says why.
    EngineFailed,
    SignedOut,
    /// Waiting for the user to sign in on Proton's page, at [`Snapshot::sign_in_url`].
    SigningIn,
    /// Signed in and up to date as of [`Snapshot::last_sync`].
    Idle,
    Syncing,
    Paused,
    /// The last sync failed; [`Snapshot::error`] says why. It is tried again on the next check.
    SyncFailed,
}

/// Where the app's own updates stand.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Update {
    /// This build doesn't update itself (a development build, or an unknown platform).
    Unsupported,
    Unchecked,
    Checking,
    UpToDate,
    Available {
        version: String,
    },
    Installing {
        version: String,
        received: u64,
        total: Option<u64>,
    },
    /// The new version is in place; the app is about to start again as it.
    Restarting,
    Failed(String),
}

/// One file the last syncs touched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Activity {
    /// Seconds since the Unix epoch.
    pub when: u64,
    pub action: &'static str,
    /// Relative to the synced folder, forward slashes.
    pub path: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub phase: Phase,
    /// The Proton account signed in.
    pub account: Option<String>,
    pub local_root: PathBuf,
    /// When the last successful sync finished, in seconds since the Unix epoch.
    pub last_sync: Option<u64>,
    pub error: Option<String>,
    /// Why the last sign-in failed.
    pub sign_in_error: Option<String>,
    /// Proton's sign-in page for the sign-in under way. It is opened in the browser for the
    /// user; the UI shows it too, to open again or on another device.
    pub sign_in_url: Option<String>,
    /// Newest first.
    pub activity: Vec<Activity>,
    pub update: Update,
}

impl Snapshot {
    pub fn new(local_root: PathBuf) -> Self {
        Self {
            phase: Phase::Starting,
            account: None,
            local_root,
            last_sync: None,
            error: None,
            sign_in_error: None,
            sign_in_url: None,
            activity: Vec::new(),
            update: Update::Unchecked,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// Start signing in: on Proton's own page, in the browser. The app never sees the password.
    SignIn,
    CancelSignIn,
    SignOut,
    SyncNow,
    SetPaused(bool),
    SetPollMinutes(u32),
    RetryInstall,
    CheckForUpdate,
    /// Install the version [`Update::Available`] names, then start again as it.
    InstallUpdate,
    SetAutoUpdate(bool),
}

pub trait Backend {
    fn snapshot(&self) -> Snapshot;
    fn send(&self, command: Command);
}
