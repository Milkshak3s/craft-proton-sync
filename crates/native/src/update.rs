//! Updating the app itself from the project's GitHub releases.
//!
//! The package for this computer is downloaded from the release, checked against the release's
//! `SHA256SUMS.txt`, unpacked in the app's data folder and swapped in for the running program,
//! which then starts again. The old program is kept until the new one is in place, and put back
//! if the swap fails.

use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use craft_proton_sync_core::update::{self, BUNDLE_NAME, Package, Release};
use sha2::{Digest, Sha256};

use crate::http;
use crate::login::HIDDEN_FLAG;
use crate::paths::Paths;

/// Passed to the program an update starts, which then waits for this one to finish quitting.
pub const RESTARTED_FLAG: &str = "--restarted";

const MAX_RELEASE_JSON: u64 = 2 * 1024 * 1024;
const MAX_SUMS: u64 = 64 * 1024;
const MAX_PACKAGE: u64 = 300 * 1024 * 1024;
const MAX_UNPACKED: u64 = 600 * 1024 * 1024;

fn err(what: &str, e: impl std::fmt::Display) -> String {
    format!("{what} ({e})")
}

/// Whether this build updates itself. Development builds don't: an update would replace the
/// program cargo just built.
pub fn supported() -> bool {
    !cfg!(debug_assertions) && update::package("0.0.0", std::env::consts::OS, std::env::consts::ARCH).is_some()
}

/// The latest release, if it is newer than `current`.
pub fn check(current: &str) -> Result<Option<Release>, String> {
    let json = http::text(update::LATEST_URL, MAX_RELEASE_JSON).map_err(|e| err("couldn't check for updates", e))?;
    let release = update::parse_release(&json)?;
    Ok(update::is_newer(current, &release.version).then_some(release))
}

/// Downloads, checks and installs `release` over the running program. `progress` gets the bytes
/// received so far and the total. Afterwards the app must be started again ([`restart`]).
pub fn install(release: &Release, paths: &Paths, progress: &mut dyn FnMut(u64, Option<u64>)) -> Result<(), String> {
    let dir = paths.update();
    let result = fetch(release, &dir, progress).and_then(|(staged, kind)| {
        let exe = std::env::current_exe()
            .and_then(fs::canonicalize)
            .map_err(|e| err("couldn't find this program", e))?;
        match kind {
            Package::AppTarGz => swap_bundle(&staged, &exe),
            Package::Zip | Package::TarGz => swap_file(&staged, &exe),
        }
    });
    let _ = fs::remove_dir_all(&dir);
    result
}

/// Downloads this computer's package of `release` into `dir`, checks it against the release's
/// checksums and unpacks it. Returns where the new program (on macOS, the new bundle) is.
pub fn fetch(release: &Release, dir: &Path, progress: &mut dyn FnMut(u64, Option<u64>)) -> Result<(PathBuf, Package), String> {
    let (name, kind) = update::package(&release.version, std::env::consts::OS, std::env::consts::ARCH)
        .ok_or("there are no updates for this kind of computer")?;
    let missing = || format!("version {} has no download for this computer yet", release.version);
    let asset = release.asset(&name).ok_or_else(missing)?;
    let sums = release.asset(update::SUMS).ok_or_else(missing)?;
    let sums = http::text(&sums.url, MAX_SUMS).map_err(|e| err("couldn't download the update's checksums", e))?;
    let expected = update::checksum_for(&sums, &name).ok_or("the update has no checksum, so it wasn't installed")?;

    let _ = fs::remove_dir_all(dir);
    fs::create_dir_all(dir).map_err(|e| err("couldn't create the app's data folder", e))?;
    let archive = dir.join("package.part");
    download(&asset.url, &archive, &expected, progress)?;
    let staged = stage(&archive, kind, &dir.join("staged"))?;
    Ok((staged, kind))
}

fn download(url: &str, to: &Path, sha256: &str, progress: &mut dyn FnMut(u64, Option<u64>)) -> Result<(), String> {
    let too_big = || "the update is far larger than it should be".to_string();
    let response = http::agent().get(url).call().map_err(|e| err("couldn't download the update", e))?;
    let total = response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    if total.is_some_and(|t| t > MAX_PACKAGE) {
        return Err(too_big());
    }
    let mut reader = response.into_body().into_reader();
    let mut file = File::create(to).map_err(|e| err("couldn't save the update", e))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    let mut received = 0u64;
    loop {
        let n = reader.read(&mut buf).map_err(|e| err("the update's download was cut short", e))?;
        let Some(chunk) = buf.get(..n).filter(|c| !c.is_empty()) else {
            break;
        };
        received = received.saturating_add(n as u64);
        if received > MAX_PACKAGE {
            return Err(too_big());
        }
        hasher.update(chunk);
        file.write_all(chunk).map_err(|e| err("couldn't save the update", e))?;
        progress(received, total);
    }
    file.sync_all().map_err(|e| err("couldn't save the update", e))?;
    let digest: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if digest != sha256 {
        return Err("the update doesn't match its checksum, so it was thrown away".into());
    }
    Ok(())
}

/// Unpacks what the update needs from a checked package into `into`: the program, or on macOS
/// the whole bundle. Returns where it is.
pub fn stage(archive: &Path, kind: Package, into: &Path) -> Result<PathBuf, String> {
    let unreadable = |e: &dyn std::fmt::Display| err("the update couldn't be unpacked", e);
    let _ = fs::remove_dir_all(into);
    fs::create_dir_all(into).map_err(|e| unreadable(&e))?;
    let file = File::open(archive).map_err(|e| unreadable(&e))?;
    let binary = update::binary_name(std::env::consts::OS);
    match kind {
        Package::Zip => {
            let mut zip = zip::ZipArchive::new(file).map_err(|e| unreadable(&e))?;
            let name = zip
                .file_names()
                .find(|n| n.rsplit('/').next() == Some(binary))
                .map(str::to_string)
                .ok_or("the update has no program in it")?;
            let entry = zip.by_name(&name).map_err(|e| unreadable(&e))?;
            let staged = into.join(binary);
            write_capped(entry, &staged)?;
            Ok(staged)
        }
        Package::TarGz => {
            let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
            for entry in tar.entries().map_err(|e| unreadable(&e))? {
                let entry = entry.map_err(|e| unreadable(&e))?;
                let is_program =
                    entry.header().entry_type().is_file() && entry.path().is_ok_and(|p| p.file_name().is_some_and(|n| n == binary));
                if is_program {
                    let staged = into.join(binary);
                    write_capped(entry, &staged)?;
                    return Ok(staged);
                }
            }
            Err("the update has no program in it".into())
        }
        Package::AppTarGz => {
            let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(file));
            let mut unpacked = 0u64;
            for entry in tar.entries().map_err(|e| unreadable(&e))? {
                let mut entry = entry.map_err(|e| unreadable(&e))?;
                unpacked = unpacked.saturating_add(entry.size());
                if unpacked > MAX_UNPACKED {
                    return Err("the update is far larger than it should be".into());
                }
                // `unpack_in` refuses paths that would land outside `into`.
                entry.unpack_in(into).map_err(|e| unreadable(&e))?;
            }
            let bundle = into.join(BUNDLE_NAME);
            if bundle.join("Contents").join("MacOS").join(binary).is_file() {
                Ok(bundle)
            } else {
                Err("the update has no program in it".into())
            }
        }
    }
}

fn write_capped(from: impl Read, to: &Path) -> Result<(), String> {
    let mut out = File::create(to).map_err(|e| err("the update couldn't be unpacked", e))?;
    let copied = io::copy(&mut from.take(MAX_UNPACKED), &mut out).map_err(|e| err("the update couldn't be unpacked", e))?;
    if copied >= MAX_UNPACKED {
        return Err("the update is far larger than it should be".into());
    }
    out.sync_all().map_err(|e| err("the update couldn't be unpacked", e))
}

fn old_path(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(".old");
    path.with_file_name(name)
}

const NOT_WRITABLE: &str = "This copy of the app is somewhere it can't change. Download the new version from the releases page instead.";

/// Puts `new` where the program `exe` is. A running program can be renamed but not overwritten
/// (Windows), so the old one is moved aside first, and moved back if the new one can't be put in
/// place. The old one is deleted when the app next starts ([`clean_up`]).
pub fn swap_file(new: &Path, exe: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(new, fs::Permissions::from_mode(0o755)).map_err(|e| err("couldn't prepare the update", e))?;
    }
    let old = old_path(exe);
    let _ = fs::remove_file(&old);
    fs::rename(exe, &old).map_err(|e| err(NOT_WRITABLE, e))?;
    // Copied, not moved: the data folder may be on another disk.
    if let Err(e) = fs::copy(new, exe) {
        let _ = fs::remove_file(exe);
        let _ = fs::rename(&old, exe);
        return Err(err(NOT_WRITABLE, e));
    }
    Ok(())
}

/// Puts the bundle `new` where the bundle holding `exe` is (macOS). Outside a bundle, as when
/// run from a build folder, only the program is replaced.
fn swap_bundle(new: &Path, exe: &Path) -> Result<(), String> {
    let binary = update::binary_name(std::env::consts::OS);
    let Some(bundle) = exe.ancestors().find(|p| p.extension().is_some_and(|e| e == "app")) else {
        return swap_file(&new.join("Contents").join("MacOS").join(binary), exe);
    };
    let old = old_path(bundle);
    let _ = fs::remove_dir_all(&old);
    fs::rename(bundle, &old).map_err(|e| err(NOT_WRITABLE, e))?;
    if let Err(e) = fs::rename(new, bundle).or_else(|_| copy_dir(new, bundle)) {
        let _ = fs::remove_dir_all(bundle);
        let _ = fs::rename(&old, bundle);
        return Err(err(NOT_WRITABLE, e));
    }
    let _ = fs::remove_dir_all(&old);
    Ok(())
}

fn copy_dir(from: &Path, to: &Path) -> io::Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Removes what an earlier update left behind: the old program and any half-done download.
pub fn clean_up(paths: &Paths) {
    let _ = fs::remove_dir_all(paths.update());
    if let Ok(exe) = std::env::current_exe().and_then(fs::canonicalize) {
        let _ = fs::remove_file(old_path(&exe));
    }
}

/// Starts the program again, as it now is on disk. The new one waits for this one to quit.
pub fn restart(hidden: bool) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| err("couldn't find this program", e))?;
    let mut cmd = Command::new(exe);
    cmd.arg(RESTARTED_FLAG)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if hidden {
        cmd.arg(HIDDEN_FLAG);
    }
    cmd.spawn().map(|_| ()).map_err(|e| err("couldn't start the new version", e))
}
