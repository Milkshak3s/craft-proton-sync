//! Where the app keeps its own files, and the folder it syncs.

use std::path::{Path, PathBuf};

use craft_proton_sync_core::layout::LOCAL_DIR;
use craft_proton_sync_core::settings::Settings;

const DIR: &str = "craft-proton-sync";

#[derive(Clone, Debug)]
pub struct Paths {
    data: PathBuf,
}

impl Paths {
    /// The app's data folder: `%LOCALAPPDATA%\craft-proton-sync`,
    /// `~/Library/Application Support/craft-proton-sync` or `~/.local/share/craft-proton-sync`.
    pub fn new() -> Result<Self, String> {
        let base = dirs::data_local_dir().ok_or("couldn't find a folder for the app's data")?;
        Ok(Self { data: base.join(DIR) })
    }

    pub fn at(data: impl Into<PathBuf>) -> Self {
        Self { data: data.into() }
    }

    pub fn data(&self) -> &Path {
        &self.data
    }

    pub fn bin(&self) -> PathBuf {
        self.data.join("bin")
    }

    /// The Proton Drive CLI's own cache, data and log folder.
    pub fn cli_data(&self) -> PathBuf {
        self.data.join("proton-drive")
    }

    /// Both sides of the folder as they were when they last agreed.
    pub fn sync_state(&self) -> PathBuf {
        self.data.join("sync-state.json")
    }

    /// Where downloads land before they are moved into the synced folder.
    pub fn staging(&self) -> PathBuf {
        self.data.join("staging")
    }

    pub fn settings(&self) -> PathBuf {
        self.data.join("settings.json")
    }

    pub fn instance_lock(&self) -> PathBuf {
        self.data.join("instance.lock")
    }

    /// The synced folder: the one in the settings, else `<Documents>/ArtCraft`.
    pub fn local_root(&self, settings: &Settings) -> Result<PathBuf, String> {
        if let Some(root) = settings.local_root.as_ref().filter(|p| p.is_absolute()) {
            return Ok(root.clone());
        }
        let documents = dirs::document_dir()
            .or_else(|| dirs::home_dir().map(|h| h.join("Documents")))
            .ok_or("couldn't find your Documents folder")?;
        Ok(documents.join(LOCAL_DIR))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_folder_is_artcraft_in_documents() {
        let root = Paths::at("data").local_root(&Settings::default()).unwrap();
        assert!(root.ends_with("ArtCraft"), "{}", root.display());
        assert!(root.is_absolute());
    }

    #[test]
    fn a_folder_in_the_settings_wins_when_it_is_absolute() {
        let chosen = std::env::temp_dir().join("my-art");
        let settings = Settings {
            local_root: Some(chosen.clone()),
            ..Settings::default()
        };
        assert_eq!(Paths::at("data").local_root(&settings).unwrap(), chosen);
        let settings = Settings {
            local_root: Some(PathBuf::from("relative")),
            ..Settings::default()
        };
        assert!(Paths::at("data").local_root(&settings).unwrap().ends_with("ArtCraft"));
    }
}
