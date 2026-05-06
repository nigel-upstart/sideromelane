//! Per-folder settings persisted under `<folder-root>/.sideromelane/settings.json`.
//!
//! Settings live in the folder so a folder remains self-describing if moved
//! between machines. The schema is versioned and unknown fields round-trip
//! through [`FolderSettings::extra`] so older clients do not erase newer keys.

use std::collections::BTreeMap;
use std::fmt;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Hidden directory that holds Sideromelane metadata for a folder.
pub const FOLDER_METADATA_DIR: &str = ".sideromelane";
/// File name of the per-folder settings document.
pub const FOLDER_SETTINGS_FILE: &str = "settings.json";

const CURRENT_SETTINGS_VERSION: u32 = 1;

/// Per-folder settings that drive walker and UI behavior.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FolderSettings {
    /// Schema version for this settings document.
    #[serde(default = "default_version")]
    pub version: u32,
    /// Walker ignore configuration.
    #[serde(default)]
    pub ignore: IgnoreSettings,
    /// Forward-compatibility bag: any unknown fields the loader did not
    /// recognise are preserved here and round-tripped on save.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

impl Default for FolderSettings {
    fn default() -> Self {
        Self {
            version: CURRENT_SETTINGS_VERSION,
            ignore: IgnoreSettings::default(),
            extra: BTreeMap::new(),
        }
    }
}

/// Walker ignore configuration controlled by the user from the folder UI.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IgnoreSettings {
    /// Whether `.gitignore` files should also be honored when walking.
    #[serde(default)]
    pub honor_gitignore: bool,
    /// Whether dotfiles and dotfolders are surfaced.
    #[serde(default)]
    pub include_dotfiles: bool,
    /// Additional ignore globs applied on top of `.sideromelaneignore`.
    #[serde(default)]
    pub extra_globs: Vec<String>,
}

const fn default_version() -> u32 {
    CURRENT_SETTINGS_VERSION
}

/// Errors produced while loading or saving [`FolderSettings`].
#[derive(Debug)]
pub enum FolderSettingsError {
    /// Filesystem-level error.
    Io(io::Error),
    /// JSON deserialization or serialization error.
    Serde(serde_json::Error),
    /// The settings file was written by a newer client and may contain fields
    /// whose semantics this version does not understand.
    FutureVersion {
        /// Version found on disk.
        found: u32,
        /// Maximum version this build supports.
        supported: u32,
    },
}

impl fmt::Display for FolderSettingsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "settings io error: {error}"),
            Self::Serde(error) => write!(formatter, "settings parse error: {error}"),
            Self::FutureVersion { found, supported } => write!(
                formatter,
                "settings version {found} is newer than supported version {supported}",
            ),
        }
    }
}

impl std::error::Error for FolderSettingsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Serde(error) => Some(error),
            Self::FutureVersion { .. } => None,
        }
    }
}

impl From<io::Error> for FolderSettingsError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for FolderSettingsError {
    fn from(error: serde_json::Error) -> Self {
        Self::Serde(error)
    }
}

impl FolderSettings {
    /// Returns the absolute path of the folder metadata directory.
    #[must_use]
    pub fn metadata_dir(folder_root: &Path) -> PathBuf {
        folder_root.join(FOLDER_METADATA_DIR)
    }

    /// Returns the absolute path of the settings file for `folder_root`.
    #[must_use]
    pub fn settings_path(folder_root: &Path) -> PathBuf {
        Self::metadata_dir(folder_root).join(FOLDER_SETTINGS_FILE)
    }

    /// Loads the settings for `folder_root`, returning defaults if no settings
    /// file exists yet.
    pub fn load(folder_root: &Path) -> Result<Self, FolderSettingsError> {
        let path = Self::settings_path(folder_root);
        match fs::read(&path) {
            Ok(bytes) => {
                let settings = serde_json::from_slice::<Self>(&bytes)?;
                if settings.version > CURRENT_SETTINGS_VERSION {
                    return Err(FolderSettingsError::FutureVersion {
                        found: settings.version,
                        supported: CURRENT_SETTINGS_VERSION,
                    });
                }
                Ok(settings)
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(FolderSettingsError::Io(error)),
        }
    }

    /// Saves the settings to `<folder-root>/.sideromelane/settings.json` using
    /// an atomic temp-file rename.
    pub fn save(&self, folder_root: &Path) -> Result<(), FolderSettingsError> {
        let metadata_dir = Self::metadata_dir(folder_root);
        fs::create_dir_all(&metadata_dir)?;

        let final_path = metadata_dir.join(FOLDER_SETTINGS_FILE);
        let temp_path = metadata_dir.join(format!("{FOLDER_SETTINGS_FILE}.tmp"));

        let payload = serde_json::to_vec_pretty(self)?;

        {
            let mut file = File::create(&temp_path)?;
            file.write_all(&payload)?;
            file.sync_data()?;
        }

        fs::rename(&temp_path, &final_path)?;

        // Best-effort fsync on the parent directory so the rename hits disk.
        if let Ok(dir) = File::open(&metadata_dir) {
            let _ = dir.sync_all();
        }

        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use serde_json::json;
    use tempfile::TempDir;

    use super::{FOLDER_METADATA_DIR, FOLDER_SETTINGS_FILE, FolderSettings};

    #[test]
    fn load_returns_defaults_when_missing() {
        let dir = TempDir::new().expect("tempdir");
        let settings = FolderSettings::load(dir.path()).expect("load default");
        assert_eq!(settings.version, 1);
        assert!(!settings.ignore.honor_gitignore);
        assert!(!settings.ignore.include_dotfiles);
        assert!(settings.ignore.extra_globs.is_empty());
        assert!(settings.extra.is_empty());
    }

    #[test]
    fn save_then_load_roundtrip() {
        let dir = TempDir::new().expect("tempdir");
        let mut settings = FolderSettings::default();
        settings.ignore.honor_gitignore = true;
        settings.ignore.include_dotfiles = true;
        settings.ignore.extra_globs.push("Drafts/".into());

        settings.save(dir.path()).expect("save ok");
        let loaded = FolderSettings::load(dir.path()).expect("load ok");
        assert!(loaded.ignore.honor_gitignore);
        assert!(loaded.ignore.include_dotfiles);
        assert_eq!(loaded.ignore.extra_globs, vec!["Drafts/".to_string()]);
    }

    #[test]
    fn unknown_fields_are_preserved() {
        let dir = TempDir::new().expect("tempdir");
        let metadata_dir = dir.path().join(FOLDER_METADATA_DIR);
        std::fs::create_dir_all(&metadata_dir).expect("metadata dir");
        let raw = json!({
            "version": 1,
            "ignore": {
                "honor_gitignore": true,
                "include_dotfiles": false,
                "extra_globs": []
            },
            "future_field": {"hello": "world"}
        });
        std::fs::write(
            metadata_dir.join(FOLDER_SETTINGS_FILE),
            serde_json::to_vec_pretty(&raw).expect("serialize"),
        )
        .expect("write");

        let loaded = FolderSettings::load(dir.path()).expect("load");
        assert!(loaded.extra.contains_key("future_field"));

        loaded.save(dir.path()).expect("save preserves extra");
        let bytes = std::fs::read(metadata_dir.join(FOLDER_SETTINGS_FILE)).expect("read");
        let value: serde_json::Value = serde_json::from_slice(&bytes).expect("parse");
        assert_eq!(value.get("future_field"), Some(&json!({"hello": "world"})));
    }

    #[test]
    fn rejects_future_version() {
        let dir = TempDir::new().expect("tempdir");
        let metadata_dir = dir.path().join(FOLDER_METADATA_DIR);
        std::fs::create_dir_all(&metadata_dir).expect("metadata dir");
        let raw = json!({
            "version": 999,
            "ignore": {}
        });
        std::fs::write(
            metadata_dir.join(FOLDER_SETTINGS_FILE),
            serde_json::to_vec_pretty(&raw).expect("serialize"),
        )
        .expect("write");

        let result = FolderSettings::load(dir.path());
        assert!(matches!(
            result,
            Err(super::FolderSettingsError::FutureVersion {
                found: 999,
                supported: 1
            })
        ));
    }

    #[test]
    fn save_creates_metadata_dir() {
        let dir = TempDir::new().expect("tempdir");
        let settings = FolderSettings::default();
        settings.save(dir.path()).expect("save ok");
        let metadata_dir = dir.path().join(FOLDER_METADATA_DIR);
        assert!(metadata_dir.is_dir());
        assert!(metadata_dir.join(FOLDER_SETTINGS_FILE).is_file());
    }
}
