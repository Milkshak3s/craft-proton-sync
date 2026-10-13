//! Downloads Proton's Drive CLI on first run.
//!
//! Only the pinned release, only from Proton's own download server, and it is run only after the
//! download matches the SHA-512 compiled into this app (`core::drive`).

use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::Path;

use craft_proton_sync_core::drive::{self, Asset};
use sha2::{Digest, Sha512};

use crate::drive::Cli;
use crate::paths::Paths;

/// The CLI is about 100 MB; anything far past that is not the CLI.
const MAX_BYTES: u64 = 400 * 1024 * 1024;

fn io_err(what: &str, e: impl std::fmt::Display) -> String {
    format!("{what} ({e})")
}

/// The CLI, ready to run: the copy already installed when it is the pinned version, else a fresh
/// download. `progress` gets the bytes received so far and the total when the server says it.
pub fn ensure(paths: &Paths, progress: &mut dyn FnMut(u64, Option<u64>)) -> Result<Cli, String> {
    let asset = drive::asset_for(std::env::consts::OS, std::env::consts::ARCH).ok_or_else(|| {
        format!(
            "Proton has no Drive CLI for this computer ({} {})",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    })?;
    let bin_dir = paths.bin();
    let bin = bin_dir.join(asset.binary_name());
    let stamp = bin_dir.join("proton-drive.version");
    let ready = Cli::new(&bin, paths.cli_data());
    if bin.is_file() && fs::read_to_string(&stamp).is_ok_and(|v| v.trim() == drive::VERSION) {
        return Ok(ready);
    }
    fs::create_dir_all(&bin_dir).map_err(|e| io_err("couldn't create the app's data folder", e))?;
    let staged = bin_dir.join("proton-drive.part");
    let result = download(asset, &staged, progress).and_then(|()| install(&staged, &bin));
    let _ = fs::remove_file(&staged);
    result?;
    fs::write(&stamp, drive::VERSION).map_err(|e| io_err("couldn't finish installing the Proton Drive CLI", e))?;
    log::info!("installed the Proton Drive CLI {} at {}", drive::VERSION, bin.display());
    Ok(ready)
}

fn download(asset: &Asset, to: &Path, progress: &mut dyn FnMut(u64, Option<u64>)) -> Result<(), String> {
    let too_big = || "the Proton Drive CLI download is far larger than it should be".to_string();
    let response = crate::http::agent()
        .get(asset.url())
        .call()
        .map_err(|e| io_err("couldn't download the Proton Drive CLI", e))?;
    let total = response
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok());
    if total.is_some_and(|t| t > MAX_BYTES) {
        return Err(too_big());
    }
    let mut reader = response.into_body().into_reader();
    let mut file = File::create(to).map_err(|e| io_err("couldn't save the Proton Drive CLI download", e))?;
    let mut hasher = Sha512::new();
    let mut buf = vec![0u8; 256 * 1024];
    let mut received = 0u64;
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|e| io_err("the Proton Drive CLI download was cut short", e))?;
        let Some(chunk) = buf.get(..n).filter(|c| !c.is_empty()) else {
            break;
        };
        received = received.saturating_add(n as u64);
        if received > MAX_BYTES {
            return Err(too_big());
        }
        hasher.update(chunk);
        file.write_all(chunk)
            .map_err(|e| io_err("couldn't save the Proton Drive CLI download", e))?;
        progress(received, total);
    }
    file.sync_all()
        .map_err(|e| io_err("couldn't save the Proton Drive CLI download", e))?;
    let digest: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if digest != asset.sha512 {
        return Err("the Proton Drive CLI download doesn't match its checksum, so it was thrown away".into());
    }
    Ok(())
}

/// Puts the checked download in place, replacing any older copy.
fn install(staged: &Path, bin: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(staged, fs::Permissions::from_mode(0o755)).map_err(|e| io_err("couldn't install the Proton Drive CLI", e))?;
    }
    let _ = fs::remove_file(bin);
    fs::rename(staged, bin).map_err(|e| io_err("couldn't install the Proton Drive CLI", e))
}
