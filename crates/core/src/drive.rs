//! Proton's own Drive CLI, which does the talking to Proton Drive: which release to download,
//! the command lines it is given and how its answers read.
//!
//! The CLI is built by Proton on the SDK their own apps use. The release is pinned and its
//! checksums are compiled in. It has no sync command: [`crate::plan`] decides what to do and the
//! CLI lists, uploads, downloads and trashes.

use serde_json::Value;

use crate::layout::REMOTE_ROOT;
use crate::plan::RemoteStamp;

/// The pinned CLI release. Bump it together with every checksum in [`ASSETS`].
pub const VERSION: &str = "0.9.0";
/// Proton's download server; the only place the CLI is fetched from.
const DOWNLOADS: &str = "https://proton.me/download/drive/cli";
/// The user's own files in Proton Drive, as the CLI names them.
pub const MY_FILES: &str = "/my-files";
/// What the CLI prints when it is ready for the next command in its interactive mode.
pub const PROMPT: &str = "proton-drive> ";

/// One CLI build, named the way Proton names its platforms.
#[derive(Debug, PartialEq, Eq)]
pub struct Asset {
    pub platform: &'static str,
    /// From `https://proton.me/download/drive/cli/version.json`.
    pub sha512: &'static str,
}

const ASSETS: [Asset; 6] = [
    Asset {
        platform: "darwin-arm64",
        sha512: "3f4125de5e41dcc7f4e0e07e8d60737aa18f35c6ecc1ba3fce47ea97b331857eefaf88e74671135af57122d4cb5439b817ec528f8df383550ac7810aeaf0217b",
    },
    Asset {
        platform: "darwin-x64",
        sha512: "a9fc830f38b680ef537fd92db75e1d6e345374f977efa523b145ccc7ec98570756f6ca32378d1bad6a84c9542f24ef43a63c6ec956e5dda9ca731b60b001a6e6",
    },
    Asset {
        platform: "linux-arm64",
        sha512: "c8d5a6b174e57f06d05cb5483400b9963faf58e743b6b33706bb73ecff718047ffce6b439994255b94b5b226b90d2e91bef7418ec96c1c3f5d8bf39b6cd01321",
    },
    Asset {
        platform: "linux-x64",
        sha512: "3533025ba69ae112b64e3e01fbcc1ad0688136a4043f6cf6a72886967d85fdcd9ec235479c2e113171614be5225bfba93427509a05ca5aa6071d924fa7e91ca8",
    },
    Asset {
        platform: "windows-arm64",
        sha512: "dbb64152dfb3b180f3f49c3e0a33d7daf159b42b041e0aa83b63fd008cadac4a7bedd252014c426e0a9c06a2c5d355b88a697acc2dcfc32543ecaa6cad9cc0fd",
    },
    Asset {
        platform: "windows-x64",
        sha512: "c68f2725b21bd8eddf83b816a059beeab4f1d2f0bb459e6bb53d1b1b972ea971e9d9f191d9cb2aa41f6560c04936aa60f28bb6382191cb0e4e51dd41d36c8bed",
    },
];

/// The CLI build for a platform, named as in `std::env::consts::{OS, ARCH}`.
pub fn asset_for(os: &str, arch: &str) -> Option<&'static Asset> {
    let os = match os {
        "windows" => "windows",
        "macos" => "darwin",
        "linux" => "linux",
        _ => return None,
    };
    let arch = match arch {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        _ => return None,
    };
    let platform = format!("{os}-{arch}");
    ASSETS.iter().find(|a| a.platform == platform)
}

impl Asset {
    pub fn binary_name(&self) -> &'static str {
        if self.platform.starts_with("windows") {
            "proton-drive.exe"
        } else {
            "proton-drive"
        }
    }

    pub fn url(&self) -> String {
        format!("{DOWNLOADS}/{VERSION}/{}/{}", self.platform, self.binary_name())
    }
}

/// Whether a file or folder name can be synced: usable on every desktop and in a CLI command
/// line. Anything else is left alone on the side it is on.
pub fn name_ok(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.ends_with([' ', '.'])
        && !name
            .chars()
            .any(|c| c.is_control() || matches!(c, '/' | '\\' | '"' | '<' | '>' | ':' | '|' | '?' | '*'))
}

/// The Proton Drive path of `relative` (forward slashes, under the synced folder; empty for the
/// folder itself).
pub fn remote_path(relative: &str) -> String {
    if relative.is_empty() {
        format!("{MY_FILES}/{REMOTE_ROOT}")
    } else {
        format!("{MY_FILES}/{REMOTE_ROOT}/{relative}")
    }
}

/// One argument in double quotes for the CLI's interactive mode; `None` when it can't be quoted.
fn quote(arg: &str) -> Option<String> {
    if arg.chars().any(|c| c.is_control() || c == '"') {
        return None;
    }
    Some(format!("\"{arg}\""))
}

/// A command line for the CLI's interactive mode; `None` when an argument can't be passed.
fn line(command: &str, args: &[&str]) -> Option<String> {
    let mut out = String::from(command);
    for arg in args {
        out.push(' ');
        out.push_str(&quote(arg)?);
    }
    out.push_str(" --json");
    Some(out)
}

pub fn list(relative: &str) -> Option<String> {
    line("filesystem list", &[&remote_path(relative)])
}

pub fn list_my_files() -> Option<String> {
    line("filesystem list", &[MY_FILES])
}

pub fn info(relative: &str) -> Option<String> {
    line("filesystem info", &[&remote_path(relative)])
}

/// Creates the synced folder itself in the user's files.
pub fn create_root() -> Option<String> {
    line("filesystem create-folder", &[MY_FILES, REMOTE_ROOT])
}

pub fn create_folder(parent: &str, name: &str) -> Option<String> {
    line("filesystem create-folder", &[&remote_path(parent), name])
}

/// Uploads a file into the folder `parent`; over a file of the same name it becomes a new
/// revision, so the old content stays in the file's history.
pub fn upload(local: &str, parent: &str) -> Option<String> {
    line("filesystem upload -f create-new-revision -d merge", &[local, &remote_path(parent)])
}

/// Downloads a file into the local folder `into`, under its own name.
pub fn download(relative: &str, into: &str) -> Option<String> {
    line("filesystem download -f remove -d merge", &[&remote_path(relative), into])
}

/// Moves a file to Proton Drive's trash, where it can be restored from.
pub fn trash(relative: &str) -> Option<String> {
    line("filesystem trash", &[&remote_path(relative)])
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Kind {
    Folder,
    File {
        stamp: RemoteStamp,
        /// The SHA-1 of the content, as the uploader claimed it.
        sha1: Option<String>,
    },
}

/// A file or folder in Proton Drive.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Node {
    pub name: String,
    pub kind: Kind,
}

fn node(value: &Value) -> Option<Node> {
    // A name that couldn't be decrypted has `ok: false`.
    let name = value
        .get("name")
        .filter(|n| n.get("ok").and_then(Value::as_bool) == Some(true))?
        .get("value")?
        .as_str()?;
    let kind = match value.get("type")?.as_str()? {
        "folder" => Kind::Folder,
        "file" => {
            let revision = value.get("activeRevision")?;
            let size = revision
                .get("claimedSize")
                .or_else(|| value.get("totalStorageSize"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
            let sha1 = revision
                .get("claimedDigests")
                .and_then(|d| d.get("sha1"))
                .and_then(Value::as_str)
                .map(str::to_lowercase);
            Kind::File {
                stamp: RemoteStamp {
                    size,
                    version: revision.get("uid")?.as_str()?.to_string(),
                },
                sha1,
            }
        }
        // Albums, photos, Proton Docs: not files the Craft apps can open.
        _ => return None,
    };
    Some(Node {
        name: name.to_string(),
        kind,
    })
}

fn json(output: &str) -> Result<Value, String> {
    serde_json::from_str(output.trim()).map_err(|_| "Proton Drive gave an answer that couldn't be read".to_string())
}

/// The files and folders in the answer to [`list`]. Entries that aren't plain files or folders,
/// or whose names can't be read, are left out.
pub fn parse_list(output: &str) -> Result<Vec<Node>, String> {
    match json(output)? {
        Value::Array(items) => Ok(items.iter().filter_map(node).collect()),
        _ => Err("Proton Drive gave an answer that couldn't be read".into()),
    }
}

/// The file in the answer to [`info`].
pub fn parse_info(output: &str) -> Result<Node, String> {
    node(&json(output)?).ok_or_else(|| "Proton Drive gave an answer that couldn't be read".to_string())
}

/// The account that owns the folder in the answer to [`info`].
pub fn owner(output: &str) -> Option<String> {
    json(output).ok()?.get("ownedBy")?.get("email")?.as_str().map(str::to_string)
}

/// Whether an upload or download moved its file. The CLI answers even a failed transfer calmly.
pub fn parse_transfer(output: &str) -> Result<(), String> {
    let answer = json(output)?;
    let count = |key: &str| answer.get(key).and_then(Value::as_u64).unwrap_or(0);
    if count("failedItems") > 0 {
        let reason = answer
            .get("failures")
            .and_then(Value::as_array)
            .and_then(|f| f.first())
            .map(|f| match f {
                Value::String(s) => s.clone(),
                other => other
                    .get("error")
                    .or_else(|| other.get("message"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| other.to_string()),
            });
        return Err(friendly(&reason.unwrap_or_else(|| "the transfer failed".into())));
    }
    if count("transferredItems") + count("skippedItems") == 0 {
        return Err("nothing was transferred".into());
    }
    Ok(())
}

/// Whether [`trash`] trashed everything it was asked to.
pub fn parse_trash(output: &str) -> Result<(), String> {
    match json(output)? {
        Value::Array(items) if items.iter().all(|i| i.get("ok").and_then(Value::as_bool) == Some(true)) => Ok(()),
        _ => Err("Proton Drive couldn't move the file to its trash".into()),
    }
}

/// The page to sign in on, from a line `auth login` printed. Only Proton's own account site.
pub fn sign_in_url(line: &str) -> Option<&str> {
    let url = line.trim();
    (url.starts_with("https://account.proton.me/") && !url.contains(char::is_whitespace)).then_some(url)
}

/// Whether the CLI refused a command because nobody is signed in.
pub fn signed_out(error: &str) -> bool {
    let e = error.to_lowercase();
    e.contains("need to login") || e.contains("need to log in") || e.contains("session expired") || e.contains("invalid refresh token")
}

pub fn not_found(error: &str) -> bool {
    error.to_lowercase().contains("node not found")
}

pub fn already_exists(error: &str) -> bool {
    error.to_lowercase().contains("already exists")
}

const MAX_ERROR_CHARS: usize = 300;

/// What the CLI printed when a command failed, in words the user can act on.
pub fn friendly(error: &str) -> String {
    let e = error.to_lowercase();
    if [
        "enotfound",
        "econnrefused",
        "etimedout",
        "network",
        "fetch failed",
        "unable to connect",
        "offline",
    ]
    .iter()
    .any(|n| e.contains(n))
    {
        return "Couldn't reach Proton Drive. It will be tried again.".into();
    }
    if e.contains("storage") && (e.contains("quota") || e.contains("full") || e.contains("exceeded")) {
        return "Your Proton Drive storage is full.".into();
    }
    match error.lines().map(str::trim).rfind(|l| !l.is_empty()) {
        Some(line) => line.chars().take(MAX_ERROR_CHARS).collect(),
        None => "Proton Drive didn't answer".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_desktop_platform_has_a_build() {
        for (os, arch, url) in [
            (
                "windows",
                "x86_64",
                "https://proton.me/download/drive/cli/0.9.0/windows-x64/proton-drive.exe",
            ),
            (
                "windows",
                "aarch64",
                "https://proton.me/download/drive/cli/0.9.0/windows-arm64/proton-drive.exe",
            ),
            (
                "macos",
                "x86_64",
                "https://proton.me/download/drive/cli/0.9.0/darwin-x64/proton-drive",
            ),
            (
                "macos",
                "aarch64",
                "https://proton.me/download/drive/cli/0.9.0/darwin-arm64/proton-drive",
            ),
            (
                "linux",
                "x86_64",
                "https://proton.me/download/drive/cli/0.9.0/linux-x64/proton-drive",
            ),
            (
                "linux",
                "aarch64",
                "https://proton.me/download/drive/cli/0.9.0/linux-arm64/proton-drive",
            ),
        ] {
            let asset = asset_for(os, arch).unwrap();
            assert_eq!(asset.url(), url);
            assert_eq!(asset.sha512.len(), 128);
            assert!(asset.sha512.bytes().all(|b| b.is_ascii_hexdigit()));
        }
        assert_eq!(asset_for("freebsd", "x86_64"), None);
        assert_eq!(asset_for("linux", "riscv64"), None);
    }

    #[test]
    fn commands_quote_their_paths() {
        assert_eq!(list("").unwrap(), r#"filesystem list "/my-files/ArtCraft" --json"#);
        assert_eq!(
            list("PhotoCraft/my holiday").unwrap(),
            r#"filesystem list "/my-files/ArtCraft/PhotoCraft/my holiday" --json"#
        );
        assert_eq!(create_root().unwrap(), r#"filesystem create-folder "/my-files" "ArtCraft" --json"#);
        assert_eq!(
            create_folder("", "PhotoCraft").unwrap(),
            r#"filesystem create-folder "/my-files/ArtCraft" "PhotoCraft" --json"#
        );
        assert_eq!(
            upload(r"C:\Users\me\Documents\ArtCraft\PhotoCraft\beach.pcraft", "PhotoCraft").unwrap(),
            r#"filesystem upload -f create-new-revision -d merge "C:\Users\me\Documents\ArtCraft\PhotoCraft\beach.pcraft" "/my-files/ArtCraft/PhotoCraft" --json"#
        );
        assert_eq!(
            download("FilmCraft/cut.fcraft", "/tmp/staging").unwrap(),
            r#"filesystem download -f remove -d merge "/my-files/ArtCraft/FilmCraft/cut.fcraft" "/tmp/staging" --json"#
        );
        assert_eq!(trash("a.txt").unwrap(), r#"filesystem trash "/my-files/ArtCraft/a.txt" --json"#);
    }

    #[test]
    fn a_name_that_would_break_out_of_its_quotes_is_refused() {
        assert_eq!(list("a\" --json\nauth logout"), None);
        assert_eq!(upload("evil\".txt", ""), None);
        assert_eq!(trash("line\nbreak"), None);
    }

    #[test]
    fn only_portable_names_are_synced() {
        for name in [
            "beach.pcraft",
            "we ird 'n(1) é#.txt",
            ".hidden",
            "beach (conflict 2026-10-12 2104).pcraft",
        ] {
            assert!(name_ok(name), "{name}");
        }
        for name in [
            "",
            ".",
            "..",
            "a/b",
            "a\\b",
            "say \"hi\"",
            "what?.txt",
            "a:b",
            "tab\there",
            "trailing.",
            "trailing ",
        ] {
            assert!(!name_ok(name), "{name:?}");
        }
    }

    const LISTING: &str = r#"[
{"uid":"v~folder","name":{"ok":true,"value":"CADCraft"},"type":"folder","folder":{"isImported":false}},
{"uid":"v~file","name":{"ok":true,"value":"About this folder.txt"},"ownedBy":{"email":"artist@proton.me"},"type":"file","totalStorageSize":367,
 "activeRevision":{"uid":"v~file~rev1","state":"active","claimedSize":314,"claimedModificationTime":"2026-10-13T04:23:15.000Z","claimedDigests":{"sha1":"A81A971B702A0608F1B3A782363DFE087D02A72C","sha1Verified":false}}},
{"uid":"v~secret","name":{"ok":false,"error":"cannot decrypt"},"type":"file"},
{"uid":"v~album","name":{"ok":true,"value":"Holiday"},"type":"album"}
]"#;

    #[test]
    fn a_listing_reads_as_files_and_folders() {
        let nodes = parse_list(LISTING).unwrap();
        assert_eq!(
            nodes,
            [
                Node {
                    name: "CADCraft".into(),
                    kind: Kind::Folder
                },
                Node {
                    name: "About this folder.txt".into(),
                    kind: Kind::File {
                        stamp: RemoteStamp {
                            size: 314,
                            version: "v~file~rev1".into()
                        },
                        sha1: Some("a81a971b702a0608f1b3a782363dfe087d02a72c".into())
                    }
                },
            ]
        );
        assert_eq!(parse_list("[\n\n]\n").unwrap(), []);
        assert!(parse_list("").is_err());
        assert!(parse_list("{}").is_err());
    }

    #[test]
    fn info_gives_the_file_and_its_owner() {
        let one = r#"{"uid":"v~f","name":{"ok":true,"value":"root"},"ownedBy":{"email":"artist@proton.me"},"type":"folder"}"#;
        assert_eq!(
            parse_info(one).unwrap(),
            Node {
                name: "root".into(),
                kind: Kind::Folder
            }
        );
        assert_eq!(owner(one).as_deref(), Some("artist@proton.me"));
        assert_eq!(owner("not json"), None);
    }

    #[test]
    fn a_transfer_counts_only_when_something_moved_and_nothing_failed() {
        assert!(parse_transfer(r#"{"transferredItems":1,"transferredBytes":9,"skippedItems":0,"failedItems":0,"failures":[]}"#).is_ok());
        // Same content already there.
        assert!(parse_transfer(r#"{"transferredItems":0,"transferredBytes":0,"skippedItems":1,"failedItems":0,"failures":[]}"#).is_ok());
        let failed = parse_transfer(
            r#"{"transferredItems":0,"skippedItems":0,"failedItems":1,"failures":[{"path":"a","error":"Disk is on fire"}]}"#,
        );
        assert_eq!(failed.unwrap_err(), "Disk is on fire");
        assert!(parse_transfer(r#"{"transferredItems":0,"skippedItems":0,"failedItems":0,"failures":[]}"#).is_err());
        assert!(parse_transfer("").is_err());
    }

    #[test]
    fn trashing_must_be_confirmed_for_every_file() {
        assert!(parse_trash(r#"[{"uid":"a","ok":true}]"#).is_ok());
        assert!(parse_trash(r#"[{"uid":"a","ok":true},{"uid":"b","ok":false}]"#).is_err());
        assert!(parse_trash("").is_err());
    }

    #[test]
    fn only_protons_account_site_is_a_sign_in_page() {
        let url = "https://account.proton.me/desktop/login?app=drive&pv=3#payload=0%3AB5P0%3Acli-drive";
        assert_eq!(sign_in_url(&format!("{url}\r\n")), Some(url));
        assert_eq!(sign_in_url("Open following URL manually (can be on another device):"), None);
        assert_eq!(sign_in_url("https://account.proton.me.evil.example/login"), None);
        assert_eq!(sign_in_url("https://evil.example/?https://account.proton.me/"), None);
        assert_eq!(sign_in_url("https://account.proton.me/x && calc.exe"), None);
    }

    #[test]
    fn errors_are_told_apart() {
        assert!(signed_out("You need to login first\n"));
        assert!(!signed_out("Node not found: nope"));
        assert!(not_found("Node not found: nope"));
        assert!(already_exists("A file or folder with that name already exists"));
        assert_eq!(friendly("\nNode not found: nope\n"), "Node not found: nope");
        assert!(friendly("TypeError: fetch failed").contains("tried again"));
        assert_eq!(friendly(""), "Proton Drive didn't answer");
    }
}
