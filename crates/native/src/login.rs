//! "Start Craft Proton Sync when I log in".
//!
//! - macOS: a LaunchAgent in `~/Library/LaunchAgents`.
//! - Windows: a value under `HKCU\…\CurrentVersion\Run`.
//! - Linux: an XDG autostart entry in `~/.config/autostart`.
//!
//! Started this way (`--hidden`) the app goes straight to the tray.

use std::path::PathBuf;

#[cfg_attr(windows, allow(dead_code))]
const ID: &str = "craft-proton-sync";
#[cfg_attr(not(windows), allow(dead_code))]
const NAME: &str = "Craft Proton Sync";
/// The command-line flag a login start passes.
pub const HIDDEN_FLAG: &str = "--hidden";

/// The program to start at login: the AppImage when running from one, else this executable.
fn program() -> Result<PathBuf, String> {
    if let Some(appimage) = std::env::var_os("APPIMAGE") {
        return Ok(PathBuf::from(appimage));
    }
    std::env::current_exe().map_err(|e| format!("couldn't find this program ({e})"))
}

/// The LaunchAgent that starts the app at login.
#[cfg(target_os = "macos")]
fn entry() -> Result<PathBuf, String> {
    Ok(dirs::home_dir()
        .ok_or("no home folder")?
        .join("Library/LaunchAgents")
        .join(format!("{ID}.plist")))
}

#[cfg(target_os = "macos")]
fn contents(exe: &std::path::Path) -> String {
    let exe = exe
        .display()
        .to_string()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n<dict>\n  <key>Label</key>\n  <string>{ID}</string>\n  <key>ProgramArguments</key>\n  <array>\n    <string>{exe}</string>\n    <string>{HIDDEN_FLAG}</string>\n  </array>\n  <key>RunAtLoad</key>\n  <true/>\n</dict>\n</plist>\n"
    )
}

/// The XDG autostart entry that starts the app at login.
#[cfg(not(any(windows, target_os = "macos")))]
fn entry() -> Result<PathBuf, String> {
    Ok(dirs::config_dir()
        .ok_or("no config folder")?
        .join("autostart")
        .join(format!("{ID}.desktop")))
}

#[cfg(not(any(windows, target_os = "macos")))]
fn contents(exe: &std::path::Path) -> String {
    let exe = exe.display().to_string().replace('\\', "\\\\").replace('"', "\\\"");
    format!(
        "[Desktop Entry]\nType=Application\nName=Craft Proton Sync\nExec=\"{exe}\" {HIDDEN_FLAG}\nIcon={ID}\nX-GNOME-Autostart-enabled=true\n"
    )
}

#[cfg(windows)]
fn run_key() -> Result<winreg::RegKey, String> {
    winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
        .create_subkey(r"Software\Microsoft\Windows\CurrentVersion\Run")
        .map(|(key, _)| key)
        .map_err(|e| format!("couldn't open the startup settings ({e})"))
}

pub fn is_enabled() -> bool {
    #[cfg(windows)]
    return run_key().is_ok_and(|k| k.get_value::<String, _>(NAME).is_ok());
    #[cfg(not(windows))]
    return entry().is_ok_and(|p| p.exists());
}

pub fn set_enabled(on: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        let key = run_key()?;
        if on {
            let command = format!("\"{}\" {HIDDEN_FLAG}", program()?.display());
            key.set_value(NAME, &command)
                .map_err(|e| format!("couldn't change the startup settings ({e})"))
        } else {
            match key.delete_value(NAME) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("couldn't change the startup settings ({e})")),
                _ => Ok(()),
            }
        }
    }
    #[cfg(not(windows))]
    {
        let path = entry()?;
        if !on {
            return match std::fs::remove_file(&path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(format!("couldn't remove {} ({e})", path.display())),
                _ => Ok(()),
            };
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("couldn't create {} ({e})", dir.display()))?;
        }
        std::fs::write(&path, contents(&program()?)).map_err(|e| format!("couldn't write {} ({e})", path.display()))
    }
}
