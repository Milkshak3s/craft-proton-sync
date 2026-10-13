//! Signing in to and out of Proton Drive.
//!
//! Signing in happens on Proton's own page in the browser: this app never sees the password or
//! the two-factor code. The CLI keeps the session it gets back in the operating system's secret
//! store (Windows Credential Manager, macOS Keychain, libsecret on Linux).

use std::io::{BufRead, BufReader, Read};
use std::process::{Child, Stdio};
use std::sync::Mutex;

use craft_proton_sync_core::drive::{self, MY_FILES};

use crate::drive::Cli;
use crate::lock;

/// Shown when Proton doesn't say whose account it is.
const UNNAMED: &str = "Proton account";

/// The account signed in, if there is one. Being offline doesn't count as being signed out.
pub fn current(cli: &Cli) -> Option<String> {
    let out = cli.output(["filesystem", "info", MY_FILES, "--json"]).ok()?;
    if out.status.success() {
        return Some(drive::owner(&String::from_utf8_lossy(&out.stdout)).unwrap_or_else(|| UNNAMED.to_string()));
    }
    let error = String::from_utf8_lossy(&out.stderr);
    if drive::signed_out(&error) {
        return None;
    }
    log::warn!("couldn't ask Proton Drive whose account this is: {}", drive::friendly(&error));
    Some(UNNAMED.to_string())
}

#[derive(Debug, PartialEq, Eq)]
pub enum SignIn {
    Done,
    /// Stopped from outside: the user cancelled, or the app is quitting.
    Cancelled,
    Failed(String),
}

/// Runs the CLI's sign-in to the end. `on_url` gets Proton's sign-in page as soon as the CLI
/// names it; the CLI then waits for the user to finish there. The CLI sits in `slot` meanwhile,
/// and taking it out and killing it cancels the sign-in.
pub fn sign_in(cli: &Cli, slot: &Mutex<Option<Child>>, mut on_url: impl FnMut(&str)) -> SignIn {
    let mut cmd = cli.command();
    cmd.args(["auth", "login"]).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => return SignIn::Failed(format!("couldn't run the Proton Drive CLI ({e})")),
    };
    let (stdout, stderr) = (child.stdout.take(), child.stderr.take());
    *lock(slot) = Some(child);
    let errors = std::thread::spawn(move || {
        let mut said = String::new();
        if let Some(mut stderr) = stderr {
            let _ = stderr.read_to_string(&mut said);
        }
        said
    });
    let mut last = String::new();
    if let Some(stdout) = stdout {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some(url) = drive::sign_in_url(&line) {
                on_url(url);
            } else if !line.trim().is_empty() {
                last = line;
            }
        }
    }
    let Some(mut child) = lock(slot).take() else {
        return SignIn::Cancelled;
    };
    let said = errors.join().unwrap_or_default();
    match child.wait() {
        Ok(status) if status.success() => SignIn::Done,
        Ok(_) => SignIn::Failed(format!(
            "Proton sign-in didn't finish: {}",
            drive::friendly(if said.trim().is_empty() { &last } else { &said })
        )),
        Err(e) => SignIn::Failed(format!("couldn't wait for the Proton Drive CLI ({e})")),
    }
}

/// Signs out and clears the CLI's saved session and caches.
pub fn sign_out(cli: &Cli) -> Result<(), String> {
    let out = cli.output(["auth", "logout"])?;
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "Couldn't sign out: {}",
            drive::friendly(&String::from_utf8_lossy(&out.stderr))
        ))
    }
}
