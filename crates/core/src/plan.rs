//! Decides what a two-way sync does: pure logic over three pictures of the folder.
//!
//! - `base`: both sides as they were when they last agreed (the state file).
//! - `local`: the folder on this computer now.
//! - `remote`: the folder in Proton Drive now.
//!
//! A side "changed" when it differs from `base`. A change on one side is carried to the other;
//! changes on both sides keep both copies. Nothing here deletes for good: the executor moves
//! files to the Recycle Bin / Trash locally and to Proton Drive's trash remotely.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

/// A file on this computer, as far as telling whether it changed goes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalStamp {
    pub size: u64,
    /// Modification time in milliseconds since the Unix epoch.
    pub modified_ms: i64,
}

/// A file in Proton Drive, as far as telling whether it changed goes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteStamp {
    pub size: u64,
    /// Changes whenever the file's content does: its revision.
    pub version: String,
}

/// A file as it was on both sides when they last agreed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Synced {
    pub local: LocalStamp,
    pub remote: RemoteStamp,
}

/// Files by their path under the synced folder, with forward slashes.
pub type Tree<T> = BTreeMap<String, T>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Upload(String),
    Download(String),
    /// Deleted here: move the remote copy to Proton Drive's trash.
    TrashRemote(String),
    /// Deleted in Proton Drive: move the local copy to the Recycle Bin / Trash.
    TrashLocal(String),
    /// Both sides already hold the same file: just remember that.
    Adopt(String),
    /// Gone from both sides: stop remembering it.
    Forget(String),
    /// Changed on both sides: keep the local copy under a new name, then download.
    Conflict(String),
}

impl Action {
    pub fn path(&self) -> &str {
        match self {
            Self::Upload(p)
            | Self::Download(p)
            | Self::TrashRemote(p)
            | Self::TrashLocal(p)
            | Self::Adopt(p)
            | Self::Forget(p)
            | Self::Conflict(p) => p,
        }
    }
}

/// What to do so that both sides agree. `identical` names files that are on both sides, were
/// never synced, and were checked to hold the same content.
pub fn plan(base: &Tree<Synced>, local: &Tree<LocalStamp>, remote: &Tree<RemoteStamp>, identical: &BTreeSet<String>) -> Vec<Action> {
    let paths: BTreeSet<&String> = base.keys().chain(local.keys()).chain(remote.keys()).collect();
    let mut actions = Vec::new();
    for path in paths {
        let p = || path.clone();
        let action = match (base.get(path), local.get(path), remote.get(path)) {
            (None, None, None) => None,
            // Never synced.
            (None, Some(_), None) => Some(Action::Upload(p())),
            (None, None, Some(_)) => Some(Action::Download(p())),
            (None, Some(_), Some(_)) => Some(if identical.contains(path) {
                Action::Adopt(p())
            } else {
                Action::Conflict(p())
            }),
            // Synced before.
            (Some(_), None, None) => Some(Action::Forget(p())),
            (Some(was), Some(l), None) => Some(if *l == was.local {
                Action::TrashLocal(p())
            } else {
                Action::Upload(p())
            }),
            (Some(was), None, Some(r)) => Some(if *r == was.remote {
                Action::TrashRemote(p())
            } else {
                Action::Download(p())
            }),
            (Some(was), Some(l), Some(r)) => match (*l != was.local, *r != was.remote) {
                (false, false) => None,
                (true, false) => Some(Action::Upload(p())),
                (false, true) => Some(Action::Download(p())),
                (true, true) => Some(Action::Conflict(p())),
            },
        };
        actions.extend(action);
    }
    actions
}

/// Whether a side looks gone rather than tidied: every synced file is missing from it, and so is
/// every folder those files were in.
fn vanished<T>(base: &Tree<Synced>, files: &Tree<T>, folders: &BTreeSet<String>) -> bool {
    /// One or two missing files is ordinary deleting.
    const FEW: usize = 3;
    let folder_of = |path: &String| path.rsplit_once('/').map(|(folder, _)| folder.to_string());
    base.len() >= FEW && base.keys().all(|p| !files.contains_key(p)) && !base.keys().filter_map(folder_of).any(|f| folders.contains(&f))
}

/// Refuses a sync that would empty a side which looks gone: an unplugged disk, a renamed folder,
/// a listing gone wrong. Deleting every file while their folders stay is the user's own doing
/// and goes ahead. `local_folders` and `remote_folders` are the folders each side has now,
/// before this app creates any.
pub fn guard(
    base: &Tree<Synced>,
    local: &Tree<LocalStamp>,
    local_folders: &BTreeSet<String>,
    remote: &Tree<RemoteStamp>,
    remote_folders: &BTreeSet<String>,
) -> Result<(), String> {
    if vanished(base, local, local_folders) {
        return Err("Every synced file and its folder is missing on this computer, so nothing was deleted from Proton Drive. If you meant to delete them all, sign out and in again to start afresh.".into());
    }
    if vanished(base, remote, remote_folders) {
        return Err("Every synced file and its folder is missing from Proton Drive, so nothing was deleted on this computer. If you meant to delete them all, sign out and in again to start afresh.".into());
    }
    Ok(())
}

/// Folders one side has and the other lacks, parents before children. Folders are only ever
/// created: deleting a folder on one side does not delete it on the other.
pub fn missing_folders(have: &BTreeSet<String>, want: &BTreeSet<String>) -> Vec<String> {
    want.difference(have).cloned().collect()
}

/// The name a local file is kept under when both sides changed it: `beach.pcraft` becomes
/// `beach (conflict 2026-10-12 2104).pcraft`. `now` is seconds since the Unix epoch, in UTC.
pub fn conflict_name(path: &str, now: u64) -> String {
    let (dir, name) = match path.rsplit_once('/') {
        Some((dir, name)) => (Some(dir), name),
        None => (None, path),
    };
    // A leading dot is part of the name, not an extension.
    let (stem, ext) = match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => (stem, Some(ext)),
        _ => (name, None),
    };
    let (y, m, d) = civil_from_days((now / 86_400) as i64);
    let (hh, mm) = ((now % 86_400) / 3600, (now % 3600) / 60);
    let mut out = String::new();
    if let Some(dir) = dir {
        out.push_str(dir);
        out.push('/');
    }
    out.push_str(&format!("{stem} (conflict {y:04}-{m:02}-{d:02} {hh:02}{mm:02})"));
    if let Some(ext) = ext {
        out.push('.');
        out.push_str(ext);
    }
    out
}

/// The calendar date of a day counted from 1970-01-01 (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn l(size: u64, modified_ms: i64) -> LocalStamp {
        LocalStamp { size, modified_ms }
    }

    fn r(size: u64, version: &str) -> RemoteStamp {
        RemoteStamp {
            size,
            version: version.into(),
        }
    }

    fn tree<T: Clone>(items: &[(&str, T)]) -> Tree<T> {
        items.iter().map(|(p, v)| (p.to_string(), v.clone())).collect()
    }

    fn synced() -> Tree<Synced> {
        tree(&[(
            "a.txt",
            Synced {
                local: l(1, 10),
                remote: r(1, "v1"),
            },
        )])
    }

    fn none() -> BTreeSet<String> {
        BTreeSet::new()
    }

    #[test]
    fn nothing_changed_means_nothing_to_do() {
        assert_eq!(
            plan(&synced(), &tree(&[("a.txt", l(1, 10))]), &tree(&[("a.txt", r(1, "v1"))]), &none()),
            []
        );
    }

    #[test]
    fn new_files_cross_to_the_other_side() {
        let actions = plan(
            &Tree::new(),
            &tree(&[("new-here.txt", l(1, 1))]),
            &tree(&[("new-there.txt", r(1, "v1"))]),
            &none(),
        );
        assert_eq!(
            actions,
            [Action::Upload("new-here.txt".into()), Action::Download("new-there.txt".into())]
        );
    }

    #[test]
    fn an_edit_on_one_side_is_carried_to_the_other() {
        let edited_here = plan(&synced(), &tree(&[("a.txt", l(2, 20))]), &tree(&[("a.txt", r(1, "v1"))]), &none());
        assert_eq!(edited_here, [Action::Upload("a.txt".into())]);
        let edited_there = plan(&synced(), &tree(&[("a.txt", l(1, 10))]), &tree(&[("a.txt", r(2, "v2"))]), &none());
        assert_eq!(edited_there, [Action::Download("a.txt".into())]);
    }

    #[test]
    fn a_deletion_on_one_side_trashes_the_other() {
        let deleted_here = plan(&synced(), &Tree::new(), &tree(&[("a.txt", r(1, "v1"))]), &none());
        assert_eq!(deleted_here, [Action::TrashRemote("a.txt".into())]);
        let deleted_there = plan(&synced(), &tree(&[("a.txt", l(1, 10))]), &Tree::new(), &none());
        assert_eq!(deleted_there, [Action::TrashLocal("a.txt".into())]);
        assert_eq!(
            plan(&synced(), &Tree::new(), &Tree::new(), &none()),
            [Action::Forget("a.txt".into())]
        );
    }

    #[test]
    fn an_edit_beats_a_deletion() {
        let edited_there = plan(&synced(), &Tree::new(), &tree(&[("a.txt", r(2, "v2"))]), &none());
        assert_eq!(edited_there, [Action::Download("a.txt".into())]);
        let edited_here = plan(&synced(), &tree(&[("a.txt", l(2, 20))]), &Tree::new(), &none());
        assert_eq!(edited_here, [Action::Upload("a.txt".into())]);
    }

    #[test]
    fn edits_on_both_sides_keep_both() {
        let actions = plan(&synced(), &tree(&[("a.txt", l(2, 20))]), &tree(&[("a.txt", r(3, "v2"))]), &none());
        assert_eq!(actions, [Action::Conflict("a.txt".into())]);
    }

    #[test]
    fn unsynced_files_on_both_sides_are_adopted_only_when_identical() {
        let (local, remote) = (
            tree(&[("a.txt", l(1, 10)), ("b.txt", l(1, 10))]),
            tree(&[("a.txt", r(1, "v1")), ("b.txt", r(1, "v1"))]),
        );
        let identical = BTreeSet::from(["a.txt".to_string()]);
        assert_eq!(
            plan(&Tree::new(), &local, &remote, &identical),
            [Action::Adopt("a.txt".into()), Action::Conflict("b.txt".into())]
        );
    }

    #[test]
    fn a_side_that_vanished_is_not_obeyed_but_a_tidied_one_is() {
        let base: Tree<Synced> = ["PhotoCraft/a", "PhotoCraft/b", "FilmCraft/c"]
            .iter()
            .map(|p| {
                (
                    p.to_string(),
                    Synced {
                        local: l(1, 1),
                        remote: r(1, "v"),
                    },
                )
            })
            .collect();
        let all_local: Tree<LocalStamp> = base.keys().map(|p| (p.clone(), l(1, 1))).collect();
        let all_remote: Tree<RemoteStamp> = base.keys().map(|p| (p.clone(), r(1, "v"))).collect();
        let folders = BTreeSet::from(["PhotoCraft".to_string(), "FilmCraft".to_string()]);
        let gone = BTreeSet::new();
        assert!(guard(&base, &all_local, &folders, &all_remote, &folders).is_ok());
        // Files and folders gone together: the side is missing, not emptied.
        assert!(
            guard(&base, &Tree::new(), &gone, &all_remote, &folders)
                .unwrap_err()
                .contains("on this computer")
        );
        assert!(
            guard(&base, &all_local, &folders, &Tree::new(), &gone)
                .unwrap_err()
                .contains("missing from Proton Drive")
        );
        // Every file deleted but the folders still there: the user cleared them out.
        assert!(guard(&base, &Tree::new(), &folders, &all_remote, &folders).is_ok());
        assert!(
            guard(
                &base,
                &all_local,
                &folders,
                &Tree::new(),
                &BTreeSet::from(["FilmCraft".to_string()])
            )
            .is_ok()
        );
        // Some files gone is ordinary deleting, and so is emptying a folder of a file or two.
        assert!(guard(&base, &tree(&[("PhotoCraft/a", l(1, 1))]), &gone, &all_remote, &folders).is_ok());
        assert!(guard(&synced(), &Tree::new(), &gone, &tree(&[("a.txt", r(1, "v1"))]), &gone).is_ok());
    }

    #[test]
    fn missing_folders_come_parents_first() {
        let have = BTreeSet::from(["PhotoCraft".to_string()]);
        let want = BTreeSet::from([
            "PhotoCraft".to_string(),
            "FilmCraft/cuts/final".to_string(),
            "FilmCraft".to_string(),
            "FilmCraft/cuts".to_string(),
        ]);
        assert_eq!(
            missing_folders(&have, &want),
            ["FilmCraft", "FilmCraft/cuts", "FilmCraft/cuts/final"]
        );
    }

    #[test]
    fn a_conflict_copy_is_named_for_when_it_happened() {
        // 2026-10-12 21:04:05 UTC.
        let now = 1_791_839_045;
        assert_eq!(
            conflict_name("PhotoCraft/beach.pcraft", now),
            "PhotoCraft/beach (conflict 2026-10-12 2104).pcraft"
        );
        assert_eq!(conflict_name("notes", now), "notes (conflict 2026-10-12 2104)");
        assert_eq!(conflict_name("a/.hidden", now), "a/.hidden (conflict 2026-10-12 2104)");
        assert_eq!(conflict_name("archive.tar.gz", 0), "archive.tar (conflict 1970-01-01 0000).gz");
    }
}
