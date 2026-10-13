//! The user's settings, kept as JSON in the app's data folder.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

pub const MIN_POLL_MINUTES: u32 = 1;
pub const MAX_POLL_MINUTES: u32 = 120;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// How often Proton Drive is checked for changes made elsewhere. Local saves sync at once.
    pub poll_minutes: u32,
    pub paused: bool,
    /// Closing the window keeps syncing in the tray.
    pub close_to_tray: bool,
    /// Install new versions of the app as they are published, without asking. Off unless the
    /// user turns it on.
    pub auto_update: bool,
    /// The synced folder; `None` is `<Documents>/ArtCraft`. No UI yet: edit the settings file.
    pub local_root: Option<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            poll_minutes: 5,
            paused: false,
            close_to_tray: true,
            auto_update: false,
            local_root: None,
        }
    }
}

impl Settings {
    /// Settings read from a file, with out-of-range values pulled back in.
    pub fn sanitized(mut self) -> Self {
        self.poll_minutes = self.poll_minutes.clamp(MIN_POLL_MINUTES, MAX_POLL_MINUTES);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_fields_take_their_defaults() {
        let s: Settings = serde_json::from_str(r#"{"paused":true}"#).unwrap();
        assert_eq!(
            s,
            Settings {
                paused: true,
                ..Settings::default()
            }
        );
    }

    #[test]
    fn a_wild_poll_interval_is_clamped() {
        let s: Settings = serde_json::from_str(r#"{"poll_minutes":0}"#).unwrap();
        assert_eq!(s.sanitized().poll_minutes, MIN_POLL_MINUTES);
        let s: Settings = serde_json::from_str(r#"{"poll_minutes":100000}"#).unwrap();
        assert_eq!(s.sanitized().poll_minutes, MAX_POLL_MINUTES);
    }
}
