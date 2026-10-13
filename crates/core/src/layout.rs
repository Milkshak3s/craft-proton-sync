//! Where files go.
//!
//! A file saved to `<Documents>/ArtCraft/<App>/<name>` is synced to `/ArtCraft/<App>/<name>` in
//! Proton Drive, and back. The local tree is mirrored one to one, so the remote path of any file
//! is its path under the local root with `ArtCraft/` in front.

use std::path::{Component, Path};

/// The folder in Proton Drive ("My files") everything syncs under.
pub const REMOTE_ROOT: &str = "ArtCraft";
/// The folder in the user's Documents folder everything syncs from.
pub const LOCAL_DIR: &str = "ArtCraft";

/// One folder per app in the Craft App Suite, created on both sides.
pub const APP_FOLDERS: [&str; 12] = [
    "PhotoCraft",
    "LightCraft",
    "FilmCraft",
    "DesignCraft",
    "VectorCraft",
    "PdfCraft",
    "EffectCraft",
    "WordCraft",
    "DeckCraft",
    "GridCraft",
    "CADCraft",
    "SoundCraft",
];

/// A note left in the local root, telling the user what the folder is.
pub const README_NAME: &str = "About this folder.txt";
pub const README_TEXT: &str = "Craft Proton Sync keeps this folder in step with the ArtCraft folder in your Proton Drive.\r\n\r\n\
Save a file from a Craft app into the folder named after that app (PhotoCraft, FilmCraft, ...)\r\n\
and it is uploaded to /ArtCraft/<app>/ in Proton Drive. Changes made on your other devices come\r\n\
back down to this folder.\r\n";

/// The Proton Drive path of `file`, which is under `local_root`: forward slashes, starting with
/// [`REMOTE_ROOT`]. `None` when `file` is not under `local_root`.
pub fn remote_path(local_root: &Path, file: &Path) -> Option<String> {
    let relative = file.strip_prefix(local_root).ok()?;
    let mut out = String::from(REMOTE_ROOT);
    for part in relative.components() {
        match part {
            Component::Normal(name) => {
                out.push('/');
                out.push_str(&name.to_string_lossy());
            }
            Component::CurDir => {}
            _ => return None,
        }
    }
    Some(out)
}

/// Whether `path` is a scratch file, which is never synced and never starts a sync: editors' and
/// browsers' temporary files and the desktop's own bookkeeping. The Craft apps save through a
/// temporary file and a rename.
pub fn is_transient(path: &Path) -> bool {
    let Some(name) = path.file_name().map(|n| n.to_string_lossy().to_lowercase()) else {
        return false;
    };
    const SUFFIXES: [&str; 6] = [".partial", ".tmp", ".temp", ".swp", ".crdownload", ".part"];
    const NAMES: [&str; 3] = [".ds_store", "thumbs.db", "desktop.ini"];
    SUFFIXES.iter().any(|s| name.ends_with(s)) || NAMES.contains(&name.as_str()) || name.starts_with("~$") || name.starts_with(".~lock.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn a_saved_file_lands_under_its_apps_folder() {
        let root = PathBuf::from("home").join("Documents").join("ArtCraft");
        let file = root.join("PhotoCraft").join("holiday").join("beach.pcraft");
        assert_eq!(
            remote_path(&root, &file).as_deref(),
            Some("ArtCraft/PhotoCraft/holiday/beach.pcraft")
        );
    }

    #[test]
    fn the_root_itself_is_the_remote_root() {
        let root = PathBuf::from("home").join("ArtCraft");
        assert_eq!(remote_path(&root, &root).as_deref(), Some("ArtCraft"));
    }

    #[test]
    fn files_outside_the_root_have_no_remote_path() {
        let root = PathBuf::from("home").join("ArtCraft");
        assert_eq!(remote_path(&root, &PathBuf::from("home").join("other.txt")), None);
        assert_eq!(remote_path(&root, &root.join("..").join("other.txt")), None);
    }

    #[test]
    fn scratch_files_are_transient() {
        for name in [
            "a.tmp",
            "b.PARTIAL",
            "~$report.docx",
            ".~lock.sheet.ods#",
            ".DS_Store",
            "Thumbs.db",
            "movie.mp4.crdownload",
        ] {
            assert!(is_transient(Path::new("PhotoCraft").join(name).as_path()), "{name}");
        }
        for name in ["beach.png", "template.psd", "notes.txt", "partial.png"] {
            assert!(!is_transient(Path::new("PhotoCraft").join(name).as_path()), "{name}");
        }
    }
}
