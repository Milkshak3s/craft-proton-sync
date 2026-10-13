//! One two-way sync of the folder: look at both sides, let `core::plan` decide, carry it out.
//!
//! Nothing is deleted for good. A file deleted on one side goes to the Recycle Bin / Trash on
//! this computer or to Proton Drive's trash; a file changed on both sides is kept twice.

use std::collections::BTreeSet;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use craft_proton_sync_core::drive::{self, Kind};
use craft_proton_sync_core::layout::{self, APP_FOLDERS, README_NAME, README_TEXT};
use craft_proton_sync_core::plan::{self, Action, LocalStamp, RemoteStamp, Synced, Tree};
use serde::{Deserialize, Serialize};
use sha1::{Digest, Sha1};

use crate::drive::{Remote, RemoteError};

#[derive(Debug, PartialEq, Eq)]
pub enum SyncError {
    /// The saved Proton session no longer works.
    SessionExpired,
    /// The sync was stopped because the app is quitting.
    Stopped,
    Failed(String),
}

/// Folders nested deeper than this are not followed: a loop, not a project.
const MAX_DEPTH: usize = 64;

/// Both sides as they were when they last agreed, and which two sides those were.
#[derive(Debug, Default, Serialize, Deserialize)]
struct State {
    local_root: String,
    account: String,
    files: Tree<Synced>,
}

pub struct Syncer {
    /// The synced folder.
    pub local: PathBuf,
    /// The account signed in: another account's record of the folder is not this one's.
    pub account: String,
    /// The state file.
    pub state: PathBuf,
    /// Where downloads land before they are moved into place, so a half-downloaded file is never
    /// seen in the synced folder.
    pub staging: PathBuf,
    /// Moves a local file to the Recycle Bin / Trash.
    pub trash_local: fn(&Path) -> Result<(), String>,
}

/// Moves a file to the desktop's Recycle Bin / Trash.
pub fn recycle(path: &Path) -> Result<(), String> {
    trash::delete(path).map_err(|e| format!("couldn't move {} to the trash ({e})", path.display()))
}

fn stamp(path: &Path) -> Option<LocalStamp> {
    let meta = fs::metadata(path).ok().filter(|m| m.is_file())?;
    let modified_ms = meta
        .modified()
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())?;
    Some(LocalStamp {
        size: meta.len(),
        modified_ms,
    })
}

fn sha1_of(path: &Path) -> Option<String> {
    let mut file = fs::File::open(path).ok()?;
    let mut hasher = Sha1::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = file.read(&mut buf).ok()?;
        let Some(chunk) = buf.get(..n).filter(|c| !c.is_empty()) else {
            break;
        };
        hasher.update(chunk);
    }
    Some(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

fn join(dir: &str, name: &str) -> String {
    if dir.is_empty() {
        name.to_string()
    } else {
        format!("{dir}/{name}")
    }
}

/// The folder a path is in and its own name.
fn split(path: &str) -> (&str, &str) {
    path.rsplit_once('/').unwrap_or(("", path))
}

fn syncable(name: &str) -> bool {
    drive::name_ok(name) && !layout::is_transient(Path::new(name))
}

/// Moves a file into place, across disks if need be.
fn move_file(from: &Path, to: &Path) -> std::io::Result<()> {
    if let Some(dir) = to.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::rename(from, to).or_else(|_| fs::copy(from, to).and_then(|_| fs::remove_file(from)))
}

impl Syncer {
    fn abs(&self, relative: &str) -> PathBuf {
        relative.split('/').fold(self.local.clone(), |path, part| path.join(part))
    }

    fn load(&self) -> Tree<Synced> {
        let state: State = fs::read_to_string(&self.state)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        if state.local_root == self.local.display().to_string() && state.account == self.account {
            state.files
        } else {
            Tree::new()
        }
    }

    fn save(&self, files: &Tree<Synced>) -> Result<(), String> {
        let state = State {
            local_root: self.local.display().to_string(),
            account: self.account.clone(),
            files: files.clone(),
        };
        let text = serde_json::to_string(&state).map_err(|e| e.to_string())?;
        let staged = self.state.with_extension("json.new");
        fs::write(&staged, text)
            .and_then(|()| fs::rename(&staged, &self.state))
            .map_err(|e| format!("couldn't record the sync ({e})"))
    }

    /// Forgets what was synced, so the next sync starts from the two sides as they are: after
    /// signing out.
    pub fn forget(&self) {
        let _ = fs::remove_file(&self.state);
    }

    /// Makes the synced folder with a folder per Craft app.
    pub fn prepare(&self) -> Result<(), String> {
        let folder = |e| format!("couldn't create {} ({e})", self.local.display());
        for app in APP_FOLDERS {
            fs::create_dir_all(self.local.join(app)).map_err(folder)?;
        }
        let readme = self.local.join(README_NAME);
        if !readme.exists() {
            fs::write(&readme, README_TEXT).map_err(folder)?;
        }
        Ok(())
    }

    fn scan_local(&self) -> Result<(Tree<LocalStamp>, BTreeSet<String>), String> {
        let (mut files, mut folders) = (Tree::new(), BTreeSet::new());
        let mut pending = vec![(self.local.clone(), String::new(), 0usize)];
        while let Some((dir, relative, depth)) = pending.pop() {
            let entries = fs::read_dir(&dir).map_err(|e| format!("couldn't read {} ({e})", dir.display()))?;
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                // Links are not followed: they could point anywhere.
                let Ok(kind) = entry.file_type() else { continue };
                if kind.is_symlink() || !syncable(&name) {
                    continue;
                }
                let path = join(&relative, &name);
                if kind.is_dir() {
                    if depth < MAX_DEPTH {
                        folders.insert(path.clone());
                        pending.push((entry.path(), path, depth + 1));
                    }
                } else if let Some(stamp) = stamp(&entry.path()) {
                    files.insert(path, stamp);
                }
            }
        }
        Ok((files, folders))
    }

    #[allow(clippy::type_complexity)]
    fn scan_remote(&self, remote: &mut dyn Remote) -> Result<(Tree<(RemoteStamp, Option<String>)>, BTreeSet<String>), RemoteError> {
        let (mut files, mut folders) = (Tree::new(), BTreeSet::new());
        let mut pending = vec![(String::new(), 0usize)];
        while let Some((dir, depth)) = pending.pop() {
            for node in remote.list(&dir)? {
                if !syncable(&node.name) {
                    continue;
                }
                let path = join(&dir, &node.name);
                match node.kind {
                    Kind::Folder if depth < MAX_DEPTH => {
                        folders.insert(path.clone());
                        pending.push((path, depth + 1));
                    }
                    Kind::Folder => {}
                    Kind::File { stamp, sha1 } => {
                        files.insert(path, (stamp, sha1));
                    }
                }
            }
        }
        Ok((files, folders))
    }

    /// Runs one sync to the end. `now` is seconds since the Unix epoch; `on_activity` hears of
    /// each file as it is dealt with.
    pub fn run(&self, remote: &mut dyn Remote, now: u64, on_activity: &mut dyn FnMut(&'static str, &str)) -> Result<(), SyncError> {
        let fatal = |e: RemoteError| match e {
            RemoteError::SignedOut => SyncError::SessionExpired,
            RemoteError::Stopped => SyncError::Stopped,
            other => SyncError::Failed(other.to_string()),
        };
        let mut base = self.load();
        // Before the folder is (re)made: a synced folder that has gone missing is a disk
        // unplugged or a folder renamed, not everything deleted.
        if !base.is_empty() && !self.local.is_dir() {
            return Err(SyncError::Failed(format!(
                "The folder {} is missing, so nothing was deleted from Proton Drive. Put it back, or sign out and in again to start afresh.",
                self.local.display()
            )));
        }
        // The folders that are here before this app makes any: what tells a tidied folder from
        // a missing one.
        let found_folders = if self.local.is_dir() {
            self.scan_local().map_err(SyncError::Failed)?.1
        } else {
            BTreeSet::new()
        };
        self.prepare().map_err(SyncError::Failed)?;
        remote.ensure_root().map_err(fatal)?;
        // A listing that fails ends the sync here: a side that couldn't be read is never taken
        // for a side that was emptied.
        let (remote_files, remote_folders) = self.scan_remote(remote).map_err(fatal)?;
        let (local_files, local_folders) = self.scan_local().map_err(SyncError::Failed)?;

        let mut errors: Vec<String> = Vec::new();
        for folder in plan::missing_folders(&remote_folders, &local_folders) {
            let (parent, name) = split(&folder);
            match remote.create_folder(parent, name) {
                Ok(()) => {}
                Err(e @ (RemoteError::SignedOut | RemoteError::Stopped)) => return Err(fatal(e)),
                Err(e) => errors.push(format!("{folder}: {e}")),
            }
        }
        for folder in plan::missing_folders(&local_folders, &remote_folders) {
            if let Err(e) = fs::create_dir_all(self.abs(&folder)) {
                errors.push(format!("{folder}: {e}"));
            }
        }

        let remote_stamps: Tree<RemoteStamp> = remote_files
            .iter()
            .map(|(path, (stamp, _))| (path.clone(), stamp.clone()))
            .collect();
        // Files on both sides that were never synced are the same file only if their content is.
        let identical: BTreeSet<String> = remote_files
            .iter()
            .filter(|(path, (stamp, sha1))| {
                !base.contains_key(*path)
                    && local_files.get(*path).is_some_and(|l| l.size == stamp.size)
                    && sha1.as_ref().is_some_and(|sha1| sha1_of(&self.abs(path)).as_ref() == Some(sha1))
            })
            .map(|(path, _)| path.clone())
            .collect();
        // The note this app writes itself says nothing about whether the user's files are there.
        let users: Tree<Synced> = base
            .iter()
            .filter(|(path, _)| path.as_str() != README_NAME)
            .map(|(p, s)| (p.clone(), s.clone()))
            .collect();
        plan::guard(&users, &local_files, &found_folders, &remote_stamps, &remote_folders).map_err(SyncError::Failed)?;

        let mut stopped = None;
        for action in plan::plan(&base, &local_files, &remote_stamps, &identical) {
            match self.apply(&action, remote, &mut base, &local_files, &remote_stamps, now) {
                Ok(Some(done)) => on_activity(done, action.path()),
                Ok(None) => {}
                Err(e @ (RemoteError::SignedOut | RemoteError::Stopped)) => {
                    stopped = Some(fatal(e));
                    break;
                }
                Err(e) => {
                    log::warn!("{}: {e}", action.path());
                    errors.push(format!("{}: {e}", action.path()));
                }
            }
        }
        // What did sync is recorded even when something else failed.
        self.save(&base).map_err(SyncError::Failed)?;
        if let Some(stopped) = stopped {
            return Err(stopped);
        }
        match errors.as_slice() {
            [] => Ok(()),
            [only] => Err(SyncError::Failed(only.clone())),
            [first, rest @ ..] => Err(SyncError::Failed(format!("{first} (and {} more)", rest.len()))),
        }
    }

    /// Carries out one action and records it in `base`. Returns what was done, for the activity
    /// list; `None` for bookkeeping and for files that changed again since they were looked at
    /// (the next sync takes those).
    fn apply(
        &self,
        action: &Action,
        remote: &mut dyn Remote,
        base: &mut Tree<Synced>,
        local: &Tree<LocalStamp>,
        remotes: &Tree<RemoteStamp>,
        now: u64,
    ) -> Result<Option<&'static str>, RemoteError> {
        let path = action.path();
        let abs = self.abs(path);
        let failed = |what: &str, e: &dyn std::fmt::Display| RemoteError::Failed(format!("{what} ({e})"));
        // The file as it was when the folder was looked at; if it has changed since, leave it.
        let untouched = |seen: Option<&LocalStamp>| stamp(&abs).as_ref() == seen;
        match action {
            Action::Upload(_) => {
                let Some(seen) = local.get(path).filter(|seen| untouched(Some(seen))) else {
                    return Ok(None);
                };
                let (parent, name) = split(path);
                let uploaded = remote.upload(&abs, parent, name)?;
                base.insert(
                    path.to_string(),
                    Synced {
                        local: seen.clone(),
                        remote: uploaded,
                    },
                );
                Ok(Some("Uploaded"))
            }
            Action::Download(_) | Action::Conflict(_) => {
                let Some(theirs) = remotes.get(path) else { return Ok(None) };
                if !untouched(local.get(path)) {
                    return Ok(None);
                }
                let conflict = matches!(action, Action::Conflict(_));
                if conflict {
                    // Keep the local copy under a new name; it is uploaded by the next sync.
                    let kept = self.abs(&plan::conflict_name(path, now));
                    if kept.exists() {
                        return Ok(None);
                    }
                    fs::rename(&abs, &kept).map_err(|e| failed("couldn't keep the local copy", &e))?;
                }
                let _ = fs::remove_dir_all(&self.staging);
                fs::create_dir_all(&self.staging).map_err(|e| failed("couldn't prepare the download", &e))?;
                remote.download(path, &self.staging)?;
                let (_, name) = split(path);
                move_file(&self.staging.join(name), &abs).map_err(|e| failed("couldn't put the download in place", &e))?;
                let Some(ours) = stamp(&abs) else {
                    return Err(RemoteError::Failed("the download went missing".into()));
                };
                base.insert(
                    path.to_string(),
                    Synced {
                        local: ours,
                        remote: theirs.clone(),
                    },
                );
                Ok(Some(if conflict { "Kept both" } else { "Downloaded" }))
            }
            Action::TrashRemote(_) => {
                remote.trash(path)?;
                base.remove(path);
                Ok(Some("Deleted"))
            }
            Action::TrashLocal(_) => {
                if !untouched(base.get(path).map(|was| &was.local)) {
                    return Ok(None);
                }
                (self.trash_local)(&abs).map_err(RemoteError::Failed)?;
                base.remove(path);
                Ok(Some("Deleted"))
            }
            Action::Adopt(_) => {
                if let (Some(ours), Some(theirs)) = (local.get(path), remotes.get(path)) {
                    base.insert(
                        path.to_string(),
                        Synced {
                            local: ours.clone(),
                            remote: theirs.clone(),
                        },
                    );
                }
                Ok(None)
            }
            Action::Forget(_) => {
                base.remove(path);
                Ok(None)
            }
        }
    }
}
