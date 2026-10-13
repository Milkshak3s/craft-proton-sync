//! Reads and writes the settings file. A missing or damaged file is the defaults, never an error.

use std::fs;
use std::path::Path;

use craft_proton_sync_core::settings::Settings;

pub fn load(path: &Path) -> Settings {
    match fs::read_to_string(path) {
        Ok(text) => serde_json::from_str::<Settings>(&text)
            .map(Settings::sanitized)
            .unwrap_or_else(|e| {
                log::warn!("the settings file is damaged ({e}): using the defaults");
                Settings::default()
            }),
        Err(_) => Settings::default(),
    }
}

/// Writes through a temporary file, so a crash half-way leaves the old settings in place.
pub fn save(path: &Path, settings: &Settings) -> Result<(), String> {
    let text = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("couldn't create {} ({e})", dir.display()))?;
    }
    let staged = path.with_extension("json.new");
    fs::write(&staged, text)
        .and_then(|()| fs::rename(&staged, path))
        .map_err(|e| format!("couldn't save the settings ({e})"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_come_back_as_saved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("settings.json");
        let settings = Settings {
            poll_minutes: 15,
            paused: true,
            close_to_tray: false,
            auto_update: true,
            local_root: Some(dir.path().join("art")),
        };
        save(&path, &settings).unwrap();
        assert_eq!(load(&path), settings);
    }

    #[test]
    fn a_missing_or_damaged_file_is_the_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        assert_eq!(load(&path), Settings::default());
        fs::write(&path, "{ not json").unwrap();
        assert_eq!(load(&path), Settings::default());
    }
}
