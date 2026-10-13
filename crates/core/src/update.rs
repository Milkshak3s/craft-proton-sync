//! Updating the app itself: reading the project's latest GitHub release, telling whether it is
//! newer, and picking this computer's package and its checksum.
//!
//! Updates come only from this project's own releases page, and a package is installed only
//! after it matches the release's `SHA256SUMS.txt`.

use serde_json::Value;

/// Where the latest published release is described. Drafts and pre-releases never show here.
pub const LATEST_URL: &str = "https://api.github.com/repos/Milkshak3s/craft-proton-sync/releases/latest";
/// Every file of every release lives under this; nothing is downloaded from anywhere else.
const DOWNLOADS: &str = "https://github.com/Milkshak3s/craft-proton-sync/releases/download/";
const RELEASES: &str = "https://github.com/Milkshak3s/craft-proton-sync/releases/";
/// The release's checksum file.
pub const SUMS: &str = "SHA256SUMS.txt";
const APP: &str = "craft-proton-sync";

/// A plain `major.minor.patch` version, with or without a leading `v`. Pre-releases
/// (`0.2.0-rc.1`) are not versions anyone is updated to.
pub fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let mut parts = text.trim().trim_start_matches('v').split('.');
    let mut next = || parts.next()?.parse::<u64>().ok();
    let version = (next()?, next()?, next()?);
    parts.next().is_none().then_some(version)
}

/// Whether `candidate` is a later version than `current`.
pub fn is_newer(current: &str, candidate: &str) -> bool {
    matches!((parse_version(current), parse_version(candidate)), (Some(now), Some(new)) if new > now)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
}

/// A published release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    /// Without the tag's leading `v`.
    pub version: String,
    /// The release's page, for reading what changed.
    pub page: String,
    pub assets: Vec<Asset>,
}

impl Release {
    pub fn asset(&self, name: &str) -> Option<&Asset> {
        self.assets.iter().find(|a| a.name == name)
    }
}

/// The release GitHub describes at [`LATEST_URL`]. Files that aren't on this project's releases
/// page are left out, whatever the answer claims.
pub fn parse_release(json: &str) -> Result<Release, String> {
    let unreadable = || "GitHub's answer about the latest release couldn't be read".to_string();
    let value: Value = serde_json::from_str(json).map_err(|_| unreadable())?;
    let flag = |key: &str| value.get(key).and_then(Value::as_bool).unwrap_or(false);
    if flag("draft") || flag("prerelease") {
        return Err("the latest release isn't published yet".into());
    }
    let tag = value.get("tag_name").and_then(Value::as_str).ok_or_else(unreadable)?;
    let version = tag.trim_start_matches('v').to_string();
    parse_version(&version).ok_or_else(unreadable)?;
    let assets = value
        .get("assets")
        .and_then(Value::as_array)
        .map(|assets| {
            assets
                .iter()
                .filter_map(|a| {
                    let name = a.get("name")?.as_str()?;
                    let url = a.get("browser_download_url")?.as_str()?;
                    // The file must be exactly where a release of this project keeps it.
                    (url == format!("{DOWNLOADS}{tag}/{name}") && !name.contains(['/', '\\'])).then(|| Asset {
                        name: name.to_string(),
                        url: url.to_string(),
                        size: a.get("size").and_then(Value::as_u64).unwrap_or(0),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let page = value
        .get("html_url")
        .and_then(Value::as_str)
        .filter(|u| u.starts_with(RELEASES))
        .map_or_else(|| format!("{RELEASES}tag/{tag}"), str::to_string);
    Ok(Release { version, page, assets })
}

/// How a platform's package is packed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Package {
    /// Windows: a `.zip` holding the program.
    Zip,
    /// Linux: a `.tar.gz` holding the program.
    TarGz,
    /// macOS: a `.tar.gz` holding the whole `.app` bundle.
    AppTarGz,
}

/// The name of the release file for a platform, named as in `std::env::consts::{OS, ARCH}`, and
/// how it is packed.
pub fn package(version: &str, os: &str, arch: &str) -> Option<(String, Package)> {
    match (os, arch) {
        ("windows", "x86_64") => Some((format!("{APP}-{version}-windows-x64-portable.zip"), Package::Zip)),
        ("windows", "aarch64") => Some((format!("{APP}-{version}-windows-arm64-portable.zip"), Package::Zip)),
        ("linux", "x86_64") => Some((format!("{APP}-{version}-linux-x86_64.tar.gz"), Package::TarGz)),
        ("linux", "aarch64") => Some((format!("{APP}-{version}-linux-aarch64.tar.gz"), Package::TarGz)),
        ("macos", "x86_64" | "aarch64") => Some((format!("{APP}-{version}-macos-universal.app.tar.gz"), Package::AppTarGz)),
        _ => None,
    }
}

/// The program's file name on a platform.
pub fn binary_name(os: &str) -> &'static str {
    if os == "windows" {
        "craft-proton-sync.exe"
    } else {
        "craft-proton-sync"
    }
}

/// The name of the macOS bundle inside its package.
pub const BUNDLE_NAME: &str = "Craft Proton Sync.app";

/// The SHA-256 a release's checksum file gives for `file`, in lower-case hex.
pub fn checksum_for(sums: &str, file: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, name) = line.split_once(char::is_whitespace)?;
        let name = name.trim().trim_start_matches('*');
        (name == file && hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit())).then(|| hash.to_lowercase())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// GitHub's answer for v0.1.0, cut down to what is read.
    const LATEST: &str = r#"{
      "html_url": "https://github.com/Milkshak3s/craft-proton-sync/releases/tag/v0.1.0",
      "tag_name": "v0.1.0", "draft": false, "prerelease": false,
      "assets": [
        {"name": "craft-proton-sync-0.1.0-linux-x86_64.tar.gz", "size": 9910038,
         "browser_download_url": "https://github.com/Milkshak3s/craft-proton-sync/releases/download/v0.1.0/craft-proton-sync-0.1.0-linux-x86_64.tar.gz"},
        {"name": "craft-proton-sync-0.1.0-windows-x64-portable.zip", "size": 7193159,
         "browser_download_url": "https://github.com/Milkshak3s/craft-proton-sync/releases/download/v0.1.0/craft-proton-sync-0.1.0-windows-x64-portable.zip"},
        {"name": "SHA256SUMS.txt", "size": 563,
         "browser_download_url": "https://github.com/Milkshak3s/craft-proton-sync/releases/download/v0.1.0/SHA256SUMS.txt"},
        {"name": "evil.zip", "size": 1,
         "browser_download_url": "https://example.com/Milkshak3s/craft-proton-sync/releases/download/v0.1.0/evil.zip"},
        {"name": "other-tag.zip", "size": 1,
         "browser_download_url": "https://github.com/Milkshak3s/craft-proton-sync/releases/download/v9.9.9/other-tag.zip"}
      ]
    }"#;

    const SUMS_FILE: &str = "\
1013b2859cd4710288adabaf92026078f78eb4427ad933a275a009ed00ea10a9  craft-proton-sync-0.1.0-linux-x86_64.tar.gz
BC3D7C8DD9B147E271D6EA1944CB476F6519DE43038B47D427BD9210C91DDD50 *craft-proton-sync-0.1.0-windows-x64-portable.zip
not-a-hash  craft-proton-sync-0.1.0-macos-universal.dmg
";

    #[test]
    fn versions_compare_by_number_not_by_text() {
        assert_eq!(parse_version("v0.1.0"), Some((0, 1, 0)));
        assert_eq!(parse_version("1.20.3"), Some((1, 20, 3)));
        for odd in ["", "1.2", "1.2.3.4", "0.2.0-rc.1", "one.two.three", "v"] {
            assert_eq!(parse_version(odd), None, "{odd}");
        }
        assert!(is_newer("0.1.0", "0.1.1"));
        assert!(is_newer("0.9.0", "0.10.0"));
        assert!(is_newer("0.9.9", "v1.0.0"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.2.0", "0.1.9"));
        assert!(!is_newer("0.1.0", "0.2.0-rc.1"), "nobody is updated to a pre-release");
        assert!(!is_newer("0.1.0", "garbage"));
    }

    #[test]
    fn the_latest_release_reads_with_only_its_own_files() {
        let release = parse_release(LATEST).unwrap();
        assert_eq!(release.version, "0.1.0");
        assert_eq!(release.page, "https://github.com/Milkshak3s/craft-proton-sync/releases/tag/v0.1.0");
        let names: Vec<&str> = release.assets.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "craft-proton-sync-0.1.0-linux-x86_64.tar.gz",
                "craft-proton-sync-0.1.0-windows-x64-portable.zip",
                "SHA256SUMS.txt"
            ]
        );
        assert_eq!(release.asset(SUMS).unwrap().size, 563);
        assert_eq!(release.asset("evil.zip"), None);
    }

    #[test]
    fn drafts_pre_releases_and_nonsense_are_not_releases() {
        assert!(parse_release(&LATEST.replace(r#""draft": false"#, r#""draft": true"#)).is_err());
        assert!(parse_release(&LATEST.replace(r#""prerelease": false"#, r#""prerelease": true"#)).is_err());
        assert!(parse_release(&LATEST.replace(r#""tag_name": "v0.1.0""#, r#""tag_name": "nightly""#)).is_err());
        assert!(parse_release(r#"{"message":"API rate limit exceeded"}"#).is_err());
        assert!(parse_release("<html>").is_err());
    }

    #[test]
    fn every_platform_has_its_package() {
        assert_eq!(
            package("0.2.0", "windows", "x86_64"),
            Some(("craft-proton-sync-0.2.0-windows-x64-portable.zip".into(), Package::Zip))
        );
        assert_eq!(
            package("0.2.0", "windows", "aarch64"),
            Some(("craft-proton-sync-0.2.0-windows-arm64-portable.zip".into(), Package::Zip))
        );
        assert_eq!(
            package("0.2.0", "linux", "x86_64"),
            Some(("craft-proton-sync-0.2.0-linux-x86_64.tar.gz".into(), Package::TarGz))
        );
        assert_eq!(
            package("0.2.0", "linux", "aarch64"),
            Some(("craft-proton-sync-0.2.0-linux-aarch64.tar.gz".into(), Package::TarGz))
        );
        for arch in ["x86_64", "aarch64"] {
            assert_eq!(
                package("0.2.0", "macos", arch),
                Some(("craft-proton-sync-0.2.0-macos-universal.app.tar.gz".into(), Package::AppTarGz))
            );
        }
        assert_eq!(package("0.2.0", "freebsd", "x86_64"), None);
        assert_eq!(binary_name("windows"), "craft-proton-sync.exe");
        assert_eq!(binary_name("linux"), "craft-proton-sync");
    }

    #[test]
    fn a_checksum_is_found_by_exact_file_name() {
        assert_eq!(
            checksum_for(SUMS_FILE, "craft-proton-sync-0.1.0-linux-x86_64.tar.gz").as_deref(),
            Some("1013b2859cd4710288adabaf92026078f78eb4427ad933a275a009ed00ea10a9")
        );
        assert_eq!(
            checksum_for(SUMS_FILE, "craft-proton-sync-0.1.0-windows-x64-portable.zip").as_deref(),
            Some("bc3d7c8dd9b147e271d6ea1944cb476f6519de43038b47d427bd9210c91ddd50")
        );
        assert_eq!(
            checksum_for(SUMS_FILE, "craft-proton-sync-0.1.0-macos-universal.dmg"),
            None,
            "not a hash"
        );
        assert_eq!(checksum_for(SUMS_FILE, "linux-x86_64.tar.gz"), None, "no partial names");
        assert_eq!(checksum_for(SUMS_FILE, "missing.zip"), None);
    }
}
