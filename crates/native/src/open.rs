//! Opens a folder in the desktop's file manager, or Proton's sign-in page in the browser.

use std::path::Path;
use std::process::{Command, Stdio};

pub fn folder(path: &Path) -> Result<(), String> {
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    Command::new(program)
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("couldn't open {} ({e})", path.display()))
}

/// Opens Proton's sign-in page in the default browser. Anything that isn't a page on Proton's
/// account site is refused: the address comes from another program's output.
pub fn sign_in_page(url: &str) -> Result<(), String> {
    let url = craft_proton_sync_core::drive::sign_in_url(url).ok_or("that isn't a Proton sign-in page")?;
    // No shell on the way: the address is one argument, never parsed as a command.
    let mut cmd = if cfg!(windows) {
        let mut cmd = Command::new("rundll32");
        cmd.arg("url.dll,FileProtocolHandler");
        cmd
    } else if cfg!(target_os = "macos") {
        Command::new("open")
    } else {
        Command::new("xdg-open")
    };
    cmd.arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("couldn't open the browser ({e})"))
}
