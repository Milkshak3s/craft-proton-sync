//! Whole syncs against a folder standing in for Proton Drive: what goes up, what comes down, and
//! that nothing is ever lost.

use std::fs;
use std::path::{Path, PathBuf};

use craft_proton_sync_core::drive::{Kind, Node};
use craft_proton_sync_core::layout::README_NAME;
use craft_proton_sync_core::plan::RemoteStamp;
use craft_proton_sync_native::drive::{Remote, RemoteError};
use craft_proton_sync_native::sync::{SyncError, Syncer};
use sha1::{Digest, Sha1};

/// A folder that behaves like the synced folder in Proton Drive.
struct FolderRemote {
    root: PathBuf,
    trash: PathBuf,
    /// Fail every listing, as when offline.
    offline: bool,
    signed_out: bool,
}

fn sha1(bytes: &[u8]) -> String {
    Sha1::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

impl FolderRemote {
    fn at(&self, path: &str) -> PathBuf {
        path.split('/')
            .filter(|p| !p.is_empty())
            .fold(self.root.clone(), |p, part| p.join(part))
    }

    fn stamp(path: &Path) -> RemoteStamp {
        let bytes = fs::read(path).unwrap();
        RemoteStamp {
            size: bytes.len() as u64,
            version: format!("rev-{}", sha1(&bytes)),
        }
    }
}

impl Remote for FolderRemote {
    fn ensure_root(&mut self) -> Result<(), RemoteError> {
        if self.signed_out {
            return Err(RemoteError::SignedOut);
        }
        fs::create_dir_all(&self.root).unwrap();
        Ok(())
    }

    fn list(&mut self, dir: &str) -> Result<Vec<Node>, RemoteError> {
        if self.offline {
            return Err(RemoteError::Failed("Couldn't reach Proton Drive. It will be tried again.".into()));
        }
        let mut nodes = Vec::new();
        for entry in fs::read_dir(self.at(dir)).map_err(|_| RemoteError::NotFound)? {
            let entry = entry.unwrap();
            let name = entry.file_name().to_string_lossy().into_owned();
            let kind = if entry.file_type().unwrap().is_dir() {
                Kind::Folder
            } else {
                Kind::File {
                    stamp: Self::stamp(&entry.path()),
                    sha1: Some(sha1(&fs::read(entry.path()).unwrap())),
                }
            };
            nodes.push(Node { name, kind });
        }
        Ok(nodes)
    }

    fn create_folder(&mut self, parent: &str, name: &str) -> Result<(), RemoteError> {
        fs::create_dir_all(self.at(parent).join(name)).unwrap();
        Ok(())
    }

    fn upload(&mut self, local: &Path, parent: &str, name: &str) -> Result<RemoteStamp, RemoteError> {
        assert_eq!(local.file_name().unwrap().to_string_lossy(), name);
        let to = self.at(parent).join(name);
        fs::copy(local, &to).map_err(|e| RemoteError::Failed(e.to_string()))?;
        Ok(Self::stamp(&to))
    }

    fn download(&mut self, path: &str, into: &Path) -> Result<(), RemoteError> {
        let from = self.at(path);
        fs::copy(&from, into.join(from.file_name().unwrap())).map_err(|_| RemoteError::NotFound)?;
        Ok(())
    }

    fn trash(&mut self, path: &str) -> Result<(), RemoteError> {
        fs::create_dir_all(&self.trash).unwrap();
        let from = self.at(path);
        if from.exists() {
            fs::rename(&from, self.trash.join(from.file_name().unwrap())).unwrap();
        }
        Ok(())
    }
}

/// Stands in for the Recycle Bin: a `recycled` folder beside the synced one.
fn recycle(path: &Path) -> Result<(), String> {
    let bin = path
        .ancestors()
        .find(|p| p.ends_with("ArtCraft"))
        .unwrap()
        .with_file_name("recycled");
    fs::create_dir_all(&bin).unwrap();
    fs::rename(path, bin.join(path.file_name().unwrap())).map_err(|e| e.to_string())
}

struct World {
    _dir: tempfile::TempDir,
    local: PathBuf,
    recycled: PathBuf,
    remote: FolderRemote,
    syncer: Syncer,
    now: u64,
}

impl World {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let local = dir.path().join("Documents").join("ArtCraft");
        let remote = FolderRemote {
            root: dir.path().join("drive").join("ArtCraft"),
            trash: dir.path().join("drive").join("trash"),
            offline: false,
            signed_out: false,
        };
        let data = dir.path().join("data");
        fs::create_dir_all(&data).unwrap();
        let syncer = Syncer {
            local: local.clone(),
            account: "artist@proton.me".into(),
            state: data.join("sync-state.json"),
            staging: data.join("staging"),
            trash_local: recycle,
        };
        Self {
            recycled: dir.path().join("Documents").join("recycled"),
            _dir: dir,
            local,
            remote,
            syncer,
            now: 1_791_839_045,
        }
    }

    fn try_sync(&mut self) -> Result<Vec<(&'static str, String)>, SyncError> {
        let mut seen = Vec::new();
        self.now += 60;
        self.syncer.run(&mut self.remote, self.now, &mut |action, path| {
            seen.push((action, path.to_string()))
        })?;
        Ok(seen)
    }

    fn sync(&mut self) -> Vec<(&'static str, String)> {
        self.try_sync().unwrap()
    }

    /// Writes a file with a modification time of its own, as two saves in the same instant
    /// would otherwise look alike.
    fn save(&mut self, path: &Path, content: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
        self.now += 60;
        let when = std::time::UNIX_EPOCH + std::time::Duration::from_secs(self.now);
        fs::File::options().write(true).open(path).unwrap().set_modified(when).unwrap();
    }
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn the_first_sync_makes_a_folder_per_app_on_both_sides() {
    let mut w = World::new();
    let seen = w.sync();
    assert!(w.local.join("PhotoCraft").is_dir());
    assert!(w.remote.root.join("PhotoCraft").is_dir());
    assert!(w.remote.root.join(README_NAME).is_file());
    assert_eq!(seen, [("Uploaded", README_NAME.to_string())]);
    assert_eq!(w.sync(), [], "nothing changed, nothing to do");
}

#[test]
fn a_save_goes_up_at_the_same_path() {
    let mut w = World::new();
    w.sync();
    let (file, scratch) = (
        w.local.join("PhotoCraft").join("holiday").join("beach.pcraft"),
        w.local.join("PhotoCraft").join("beach.pcraft.tmp"),
    );
    w.save(&file, "v1");
    w.save(&scratch, "scratch");
    assert_eq!(w.sync(), [("Uploaded", "PhotoCraft/holiday/beach.pcraft".to_string())]);
    assert_eq!(read(&w.remote.root.join("PhotoCraft").join("holiday").join("beach.pcraft")), "v1");
    assert!(
        !w.remote.root.join("PhotoCraft").join("beach.pcraft.tmp").exists(),
        "scratch files stay local"
    );

    w.save(&file, "v2 is longer");
    assert_eq!(w.sync(), [("Uploaded", "PhotoCraft/holiday/beach.pcraft".to_string())]);
    assert_eq!(
        read(&w.remote.root.join("PhotoCraft").join("holiday").join("beach.pcraft")),
        "v2 is longer"
    );
}

#[test]
fn changes_made_elsewhere_come_down() {
    let mut w = World::new();
    w.sync();
    let theirs = w.remote.root.join("FilmCraft").join("reel").join("cut.fcraft");
    fs::create_dir_all(theirs.parent().unwrap()).unwrap();
    fs::write(&theirs, "from the laptop").unwrap();
    assert_eq!(w.sync(), [("Downloaded", "FilmCraft/reel/cut.fcraft".to_string())]);
    let ours = w.local.join("FilmCraft").join("reel").join("cut.fcraft");
    assert_eq!(read(&ours), "from the laptop");

    fs::write(&theirs, "edited on the laptop").unwrap();
    assert_eq!(w.sync(), [("Downloaded", "FilmCraft/reel/cut.fcraft".to_string())]);
    assert_eq!(read(&ours), "edited on the laptop");
    assert_eq!(w.sync(), []);
    assert!(!w.syncer.staging.join("cut.fcraft").exists(), "nothing is left in staging");
}

#[test]
fn a_deletion_is_a_move_to_the_trash_on_the_other_side() {
    let mut w = World::new();
    let (a, b) = (
        w.local.join("PhotoCraft").join("a.pcraft"),
        w.local.join("PhotoCraft").join("b.pcraft"),
    );
    w.save(&a, "a");
    w.save(&b, "b");
    w.sync();

    fs::remove_file(&a).unwrap();
    assert_eq!(w.sync(), [("Deleted", "PhotoCraft/a.pcraft".to_string())]);
    assert!(!w.remote.root.join("PhotoCraft").join("a.pcraft").exists());
    assert_eq!(read(&w.remote.trash.join("a.pcraft")), "a", "deleted here: in Proton Drive's trash");

    fs::remove_file(w.remote.root.join("PhotoCraft").join("b.pcraft")).unwrap();
    assert_eq!(w.sync(), [("Deleted", "PhotoCraft/b.pcraft".to_string())]);
    assert!(!b.exists());
    assert_eq!(read(&w.recycled.join("b.pcraft")), "b", "deleted there: in the Recycle Bin");
    assert_eq!(w.sync(), []);
}

#[test]
fn edits_on_both_sides_keep_both_copies() {
    let mut w = World::new();
    let ours = w.local.join("WordCraft").join("essay.wcraft");
    w.save(&ours, "draft");
    w.sync();

    w.save(&ours, "my edit");
    fs::write(w.remote.root.join("WordCraft").join("essay.wcraft"), "their edit, longer").unwrap();
    assert_eq!(w.sync(), [("Kept both", "WordCraft/essay.wcraft".to_string())]);
    assert_eq!(read(&ours), "their edit, longer");
    let kept: Vec<PathBuf> = fs::read_dir(w.local.join("WordCraft"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.to_string_lossy().contains("(conflict "))
        .collect();
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert_eq!(read(&kept[0]), "my edit");
    assert_eq!(kept[0].extension().unwrap(), "wcraft");

    // The kept copy is a new file: it goes up on the next sync, and then all is quiet.
    let seen = w.sync();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].0, "Uploaded");
    assert_eq!(w.sync(), []);
}

#[test]
fn files_already_on_both_sides_are_matched_by_content() {
    let mut w = World::new();
    w.sync();
    // The same project put on both sides by hand, and one that differs.
    let (same, differs) = (
        w.local.join("GridCraft").join("same.gcraft"),
        w.local.join("GridCraft").join("differs.gcraft"),
    );
    w.save(&same, "identical");
    w.save(&differs, "mine!");
    fs::write(w.remote.root.join("GridCraft").join("same.gcraft"), "identical").unwrap();
    fs::write(w.remote.root.join("GridCraft").join("differs.gcraft"), "yours").unwrap();
    assert_eq!(w.sync(), [("Kept both", "GridCraft/differs.gcraft".to_string())]);
    assert_eq!(read(&differs), "yours");
}

#[test]
fn losing_the_record_loses_no_files() {
    let mut w = World::new();
    let ours = w.local.join("DeckCraft").join("talk.dcraft");
    w.save(&ours, "slides");
    w.sync();
    fs::write(w.remote.root.join("DeckCraft").join("theirs.dcraft"), "other slides").unwrap();
    w.syncer.forget();
    assert_eq!(w.sync(), [("Downloaded", "DeckCraft/theirs.dcraft".to_string())]);
    assert_eq!(read(&ours), "slides");
    assert!(!w.recycled.exists() && !w.remote.trash.exists(), "a fresh start deletes nothing");
}

#[test]
fn another_accounts_record_is_not_trusted() {
    let mut w = World::new();
    let ours = w.local.join("PdfCraft").join("form.pdf");
    w.save(&ours, "form");
    w.sync();
    // Another account's empty drive must not read as "everything was deleted".
    fs::remove_dir_all(&w.remote.root).unwrap();
    w.syncer.account = "someone-else@proton.me".into();
    w.sync();
    assert_eq!(read(&ours), "form");
    assert_eq!(read(&w.remote.root.join("PdfCraft").join("form.pdf")), "form");
}

#[test]
fn a_side_that_vanished_deletes_nothing() {
    let mut w = World::new();
    for name in ["a", "b", "c"] {
        let path = w.local.join("SoundCraft").join(name);
        w.save(&path, name);
    }
    w.sync();
    // The Documents folder is gone: a disk unplugged, a folder renamed.
    fs::remove_dir_all(&w.local).unwrap();
    let Err(SyncError::Failed(why)) = w.try_sync() else {
        panic!("the sync went ahead")
    };
    assert!(why.contains("nothing was deleted from Proton Drive"), "{why}");
    assert_eq!(read(&w.remote.root.join("SoundCraft").join("a")), "a");
    assert!(!w.remote.trash.exists());
}

#[test]
fn a_drive_that_vanished_deletes_nothing_here() {
    let mut w = World::new();
    for name in ["a", "b", "c"] {
        let path = w.local.join("SoundCraft").join(name);
        w.save(&path, name);
    }
    w.sync();
    // The ArtCraft folder in Proton Drive was removed or renamed.
    fs::remove_dir_all(&w.remote.root).unwrap();
    let Err(SyncError::Failed(why)) = w.try_sync() else {
        panic!("the sync went ahead")
    };
    assert!(why.contains("nothing was deleted on this computer"), "{why}");
    assert_eq!(read(&w.local.join("SoundCraft").join("a")), "a");
    assert!(!w.recycled.exists());
}

#[test]
fn clearing_out_a_folder_is_obeyed() {
    let mut w = World::new();
    for name in ["a", "b", "c"] {
        let path = w.local.join("SoundCraft").join(name);
        w.save(&path, name);
    }
    w.sync();
    for name in ["a", "b", "c"] {
        fs::remove_file(w.local.join("SoundCraft").join(name)).unwrap();
    }
    assert_eq!(w.sync().len(), 3);
    assert!(w.remote.trash.join("a").is_file() && w.remote.trash.join("c").is_file());
    assert!(w.remote.root.join("SoundCraft").is_dir());
}

#[test]
fn being_offline_or_signed_out_changes_nothing() {
    let mut w = World::new();
    let ours = w.local.join("CADCraft").join("part.ccraft");
    w.save(&ours, "part");
    w.sync();

    w.remote.offline = true;
    let Err(SyncError::Failed(why)) = w.try_sync() else {
        panic!("the sync went ahead")
    };
    assert!(why.contains("Couldn't reach Proton Drive"), "{why}");
    assert_eq!(read(&ours), "part");
    assert!(!w.recycled.exists(), "an unreadable side is not an empty side");

    w.remote.offline = false;
    w.remote.signed_out = true;
    assert_eq!(w.try_sync(), Err(SyncError::SessionExpired));
    w.remote.signed_out = false;
    assert_eq!(w.sync(), []);
}

#[test]
fn a_file_that_fails_does_not_hold_up_the_rest() {
    let mut w = World::new();
    w.sync();
    // A folder in Proton Drive sits where the file should go: that upload fails.
    fs::create_dir_all(w.remote.root.join("LightCraft").join("blocked.lcraft")).unwrap();
    let (blocked, fine) = (
        w.local.join("LightCraft").join("blocked.lcraft"),
        w.local.join("VectorCraft").join("logo.vcraft"),
    );
    w.save(&blocked, "blocked");
    w.save(&fine, "fine");
    let Err(SyncError::Failed(why)) = w.try_sync() else {
        panic!("the failure went unreported")
    };
    assert!(why.contains("LightCraft/blocked.lcraft"), "{why}");
    assert_eq!(read(&w.remote.root.join("VectorCraft").join("logo.vcraft")), "fine");
    assert_eq!(read(&blocked), "blocked");
}
