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
    /// UI presentation preferences scoped to this folder.
    #[serde(default)]
    pub ui: UiSettings,
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
            ui: UiSettings::default(),
            extra: BTreeMap::new(),
        }
    }
}

/// UI presentation preferences scoped to a single folder.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiSettings {
    /// When true, the editor soft-wraps long lines to the central pane width.
    /// When false, the editor scrolls horizontally instead.
    #[serde(default = "default_editor_word_wrap")]
    pub editor_word_wrap: bool,
    /// Folder-relative paths whose tree node should render expanded on next open.
    /// Used by the files panel.
    #[serde(default)]
    pub tree_expanded_paths: Vec<String>,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            editor_word_wrap: default_editor_word_wrap(),
            tree_expanded_paths: Vec::new(),
        }
    }
}

const fn default_editor_word_wrap() -> bool {
    true
}

/// Walker ignore configuration controlled by the user from the folder UI.
///
/// Additional glob support beyond `.sideromelaneignore` itself is intentionally not
/// exposed in v1: users edit `.sideromelaneignore` directly. Unknown ignore-related
/// fields written by future schema versions round-trip via [`FolderSettings::extra`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IgnoreSettings {
    /// Whether `.gitignore` files should also be honored when walking.
    #[serde(default)]
    pub honor_gitignore: bool,
    /// Whether dotfiles and dotfolders are surfaced.
    #[serde(default)]
    pub include_dotfiles: bool,
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
        assert!(settings.extra.is_empty());
    }

    #[test]
    fn save_then_load_roundtrip() {
        let dir = TempDir::new().expect("tempdir");
        let mut settings = FolderSettings::default();
        settings.ignore.honor_gitignore = true;
        settings.ignore.include_dotfiles = true;

        settings.save(dir.path()).expect("save ok");
        let loaded = FolderSettings::load(dir.path()).expect("load ok");
        assert!(loaded.ignore.honor_gitignore);
        assert!(loaded.ignore.include_dotfiles);
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
                "include_dotfiles": false
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
    fn ui_defaults_are_word_wrap_on_and_no_expanded_paths() {
        let dir = TempDir::new().expect("tempdir");
        let settings = FolderSettings::load(dir.path()).expect("load default");
        assert!(settings.ui.editor_word_wrap, "word-wrap default must be on");
        assert!(settings.ui.tree_expanded_paths.is_empty());
    }

    #[test]
    fn ui_settings_round_trip_through_save_and_load() {
        let dir = TempDir::new().expect("tempdir");
        let mut settings = FolderSettings::default();
        settings.ui.editor_word_wrap = false;
        settings.ui.tree_expanded_paths.push("Cloud".into());
        settings.ui.tree_expanded_paths.push("Cloud/Q4".into());

        settings.save(dir.path()).expect("save ok");
        let loaded = FolderSettings::load(dir.path()).expect("load ok");

        assert!(!loaded.ui.editor_word_wrap);
        assert_eq!(
            loaded.ui.tree_expanded_paths,
            vec!["Cloud".to_string(), "Cloud/Q4".to_string()]
        );
    }

    #[test]
    fn legacy_settings_without_ui_key_load_with_defaults() {
        // A v1 settings file written before `ui` existed must still load.
        let dir = TempDir::new().expect("tempdir");
        let metadata_dir = dir.path().join(FOLDER_METADATA_DIR);
        std::fs::create_dir_all(&metadata_dir).expect("metadata dir");
        let raw = json!({
            "version": 1,
            "ignore": {
                "honor_gitignore": false,
                "include_dotfiles": false
            }
        });
        std::fs::write(
            metadata_dir.join(FOLDER_SETTINGS_FILE),
            serde_json::to_vec_pretty(&raw).expect("serialize"),
        )
        .expect("write");

        let loaded = FolderSettings::load(dir.path()).expect("load legacy");
        assert!(loaded.ui.editor_word_wrap);
        assert!(loaded.ui.tree_expanded_paths.is_empty());
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
