//! App-local persistent state.
//!
//! Stored at `<dirs::data_local_dir()>/sideromelane/state.json` and written
//! atomically through [`crate::io::safe_write_bytes`]. Mirrors the
//! `core::folder_settings` strict-version pattern: any document whose
//! `version` is greater than the current build's supported version is
//! rejected so a downgrade cannot silently drop or corrupt fields the
//! older binary does not understand.

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::io::safe_write_bytes;

/// Subdirectory under the platform data-local directory that owns
/// Sideromelane's app-local state.
pub const APP_STATE_DIR: &str = "sideromelane";
/// File name of the app-local state document.
pub const APP_STATE_FILE: &str = "state.json";

const CURRENT_STATE_VERSION: u32 = 1;
/// Maximum allowed byte size for `state.json`. Same 1 MiB envelope used by
/// `folder_settings` — far above any plausible user state, well below any
/// adversarial blow-up.
const MAX_STATE_BYTES: u64 = 1 << 20;

/// LRU cap for the recently-opened folders list.
pub const RECENT_FOLDERS_CAP: usize = 10;

const DEFAULT_LEFT_PANE_SPLIT_RATIO: f32 = 0.55;
const MIN_LEFT_PANE_SPLIT_RATIO: f32 = 0.1;
const MAX_LEFT_PANE_SPLIT_RATIO: f32 = 0.9;

const DEFAULT_AUTO_SAVE_DEBOUNCE_SECS: u32 = 5;
const MIN_AUTO_SAVE_DEBOUNCE_SECS: u32 = 1;
const MAX_AUTO_SAVE_DEBOUNCE_SECS: u32 = 60;

/// Default app-wide excluded-file glob patterns.
pub const DEFAULT_EXCLUDED_FILE_GLOBS: &[&str] = &[".git/**", ".DS_Store", "node_modules/**"];

/// What to open on launch.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum StartupMode {
    /// Reopen the last folder and last note, falling back to the default
    /// folder if either is missing.
    #[default]
    ReloadLast,
    /// Always boot into a fresh untitled note inside the default folder.
    NewNote,
}

/// Persistent app-local state. Schema version is always written and any
/// future-version document is rejected at load time.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppState {
    /// Schema version for this document.
    pub version: u32,
    /// Behavior on app launch.
    #[serde(default)]
    pub startup_mode: StartupMode,
    /// Folder created and opened when no last folder exists.
    #[serde(default = "default_folder")]
    pub default_folder: PathBuf,
    /// Last folder opened, if any.
    #[serde(default)]
    pub last_folder: Option<PathBuf>,
    /// Folder-relative path of the last selected note in `last_folder`.
    #[serde(default)]
    pub last_note: Option<String>,
    /// LRU of recently opened folders, capped at [`RECENT_FOLDERS_CAP`].
    #[serde(default)]
    pub recent_folders: Vec<PathBuf>,
    /// Files / Search splitter ratio, clamped to `[0.1, 0.9]`.
    #[serde(default = "default_left_pane_split_ratio")]
    pub left_pane_split_ratio: f32,
    /// Auto-save debounce in seconds, clamped to `[1, 60]`.
    #[serde(default = "default_auto_save_debounce_secs")]
    pub auto_save_debounce_secs: u32,
    /// Default word-wrap setting applied when initializing a fresh folder.
    #[serde(default = "default_word_wrap")]
    pub default_word_wrap: bool,
    /// App-wide file and folder glob patterns hidden from discovery surfaces.
    #[serde(default = "default_excluded_file_globs")]
    pub excluded_file_globs: Vec<String>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            version: CURRENT_STATE_VERSION,
            startup_mode: StartupMode::default(),
            default_folder: default_folder(),
            last_folder: None,
            last_note: None,
            recent_folders: Vec::new(),
            left_pane_split_ratio: DEFAULT_LEFT_PANE_SPLIT_RATIO,
            auto_save_debounce_secs: DEFAULT_AUTO_SAVE_DEBOUNCE_SECS,
            default_word_wrap: true,
            excluded_file_globs: default_excluded_file_globs(),
        }
    }
}

fn default_folder() -> PathBuf {
    dirs::document_dir().map_or_else(
        || {
            dirs::home_dir().map_or_else(
                || PathBuf::from("Sideromelane"),
                |home| home.join("Documents").join("Sideromelane"),
            )
        },
        |home| home.join("Sideromelane"),
    )
}

const fn default_left_pane_split_ratio() -> f32 {
    DEFAULT_LEFT_PANE_SPLIT_RATIO
}

const fn default_auto_save_debounce_secs() -> u32 {
    DEFAULT_AUTO_SAVE_DEBOUNCE_SECS
}

const fn default_word_wrap() -> bool {
    true
}

fn default_excluded_file_globs() -> Vec<String> {
    DEFAULT_EXCLUDED_FILE_GLOBS
        .iter()
        .map(|pattern| (*pattern).to_owned())
        .collect()
}

/// Parses the Preferences multiline editor into normalized glob patterns.
#[must_use]
pub fn parse_excluded_file_globs(text: &str) -> Vec<String> {
    let mut globs = Vec::new();
    for pattern in text
        .lines()
        .map(str::trim)
        .filter(|pattern| !pattern.is_empty())
    {
        if !globs.iter().any(|existing| existing == pattern) {
            globs.push(pattern.to_owned());
        }
    }
    globs
}

/// Errors produced while loading or saving [`AppState`].
#[derive(Debug)]
pub enum AppStateError {
    /// Filesystem-level error.
    Io(io::Error),
    /// JSON deserialization or serialization error.
    Serde(serde_json::Error),
    /// State file was written by a newer client.
    FutureVersion {
        /// Version found on disk.
        found: u32,
        /// Maximum version this build supports.
        supported: u32,
    },
    /// Could not resolve the platform data-local directory.
    NoDataDir,
}

impl fmt::Display for AppStateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "app state io error: {error}"),
            Self::Serde(error) => write!(formatter, "app state parse error: {error}"),
            Self::FutureVersion { found, supported } => write!(
                formatter,
                "app state version {found} is newer than supported version {supported}",
            ),
            Self::NoDataDir => write!(formatter, "no platform data-local directory available"),
        }
    }
}

impl std::error::Error for AppStateError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Serde(error) => Some(error),
            Self::FutureVersion { .. } | Self::NoDataDir => None,
        }
    }
}

impl From<io::Error> for AppStateError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<serde_json::Error> for AppStateError {
    fn from(error: serde_json::Error) -> Self {
        Self::Serde(error)
    }
}

impl AppState {
    /// Returns the platform-derived path to `state.json`.
    pub fn default_path() -> Result<PathBuf, AppStateError> {
        let base = dirs::data_local_dir().ok_or(AppStateError::NoDataDir)?;
        Ok(base.join(APP_STATE_DIR).join(APP_STATE_FILE))
    }

    /// Loads state from `path`, returning defaults if the file is missing.
    pub fn load(path: &Path) -> Result<Self, AppStateError> {
        // Check the file size before reading any bytes into memory so an
        // adversarially large state.json cannot cause an unbounded allocation.
        match fs::metadata(path) {
            Ok(meta) => {
                if meta.len() > MAX_STATE_BYTES {
                    return Err(AppStateError::Io(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "state.json too large (max 1 MiB)",
                    )));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(AppStateError::Io(error)),
        }

        let bytes = fs::read(path)?;
        let mut state = serde_json::from_slice::<Self>(&bytes)?;
        if state.version > CURRENT_STATE_VERSION {
            return Err(AppStateError::FutureVersion {
                found: state.version,
                supported: CURRENT_STATE_VERSION,
            });
        }
        state.normalize();
        Ok(state)
    }

    /// Loads from the platform default path, falling back to defaults on any
    /// recoverable error so a corrupt state.json never blocks app launch.
    pub fn load_or_default() -> Self {
        Self::default_path().map_or_else(
            |_| Self::default(),
            |path| Self::load(&path).unwrap_or_default(),
        )
    }

    /// Saves state to `path` via [`crate::io::safe_write_bytes`].
    pub fn save(&self, path: &Path) -> Result<(), AppStateError> {
        let payload = serde_json::to_vec_pretty(self)?;
        safe_write_bytes(path, &payload)?;
        Ok(())
    }

    /// Saves to the platform default path.
    pub fn save_default(&self) -> Result<(), AppStateError> {
        let path = Self::default_path()?;
        self.save(&path)
    }

    /// Push `folder` to the front of `recent_folders`, dedup, and cap at
    /// [`RECENT_FOLDERS_CAP`].
    pub fn record_folder_open(&mut self, folder: &Path) {
        self.recent_folders.retain(|existing| existing != folder);
        self.recent_folders.insert(0, folder.to_path_buf());
        if self.recent_folders.len() > RECENT_FOLDERS_CAP {
            self.recent_folders.truncate(RECENT_FOLDERS_CAP);
        }
        self.last_folder = Some(folder.to_path_buf());
    }

    /// Update the in-memory `auto_save_debounce_secs`, clamping to the
    /// supported range.
    #[allow(clippy::missing_const_for_fn)] // Ord::clamp is not yet const-stable.
    pub fn set_auto_save_debounce_secs(&mut self, secs: u32) {
        self.auto_save_debounce_secs =
            secs.clamp(MIN_AUTO_SAVE_DEBOUNCE_SECS, MAX_AUTO_SAVE_DEBOUNCE_SECS);
    }

    /// Update the in-memory `left_pane_split_ratio`, clamping to the
    /// supported range.
    pub const fn set_left_pane_split_ratio(&mut self, ratio: f32) {
        self.left_pane_split_ratio =
            ratio.clamp(MIN_LEFT_PANE_SPLIT_RATIO, MAX_LEFT_PANE_SPLIT_RATIO);
    }

    /// Clamp loaded values back into their supported ranges. Tolerates
    /// hand-edited or older documents that drifted outside the bounds.
    fn normalize(&mut self) {
        self.left_pane_split_ratio = self
            .left_pane_split_ratio
            .clamp(MIN_LEFT_PANE_SPLIT_RATIO, MAX_LEFT_PANE_SPLIT_RATIO);
        self.auto_save_debounce_secs = self
            .auto_save_debounce_secs
            .clamp(MIN_AUTO_SAVE_DEBOUNCE_SECS, MAX_AUTO_SAVE_DEBOUNCE_SECS);
        if self.recent_folders.len() > RECENT_FOLDERS_CAP {
            self.recent_folders.truncate(RECENT_FOLDERS_CAP);
        }
        self.excluded_file_globs = parse_excluded_file_globs(&self.excluded_file_globs.join("\n"));
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::float_cmp)]
mod tests {
    use serde_json::json;
    use tempfile::TempDir;

    use super::{
        APP_STATE_FILE, AppState, AppStateError, DEFAULT_EXCLUDED_FILE_GLOBS, RECENT_FOLDERS_CAP,
        StartupMode, parse_excluded_file_globs,
    };

    fn state_path(dir: &TempDir) -> std::path::PathBuf {
        dir.path().join(APP_STATE_FILE)
    }

    #[test]
    fn load_returns_defaults_when_missing() {
        let dir = TempDir::new().expect("tempdir");
        let state = AppState::load(&state_path(&dir)).expect("load default");
        assert_eq!(state.version, 1);
        assert_eq!(state.startup_mode, StartupMode::ReloadLast);
        assert!(state.last_folder.is_none());
        assert!(state.last_note.is_none());
        assert!(state.recent_folders.is_empty());
        assert!((state.left_pane_split_ratio - 0.55).abs() < f32::EPSILON);
        assert_eq!(state.auto_save_debounce_secs, 5);
        assert!(state.default_word_wrap);
        assert_eq!(state.excluded_file_globs, DEFAULT_EXCLUDED_FILE_GLOBS);
    }

    #[test]
    fn save_then_load_roundtrip() {
        let dir = TempDir::new().expect("tempdir");
        let path = state_path(&dir);
        let mut state = AppState {
            startup_mode: StartupMode::NewNote,
            last_folder: Some(dir.path().to_path_buf()),
            last_note: Some("Inbox.md".into()),
            default_word_wrap: false,
            excluded_file_globs: vec!["target/**".into(), "**/.obsidian/**".into()],
            ..AppState::default()
        };
        state.set_left_pane_split_ratio(0.42);
        state.set_auto_save_debounce_secs(15);
        state.record_folder_open(dir.path());

        state.save(&path).expect("save ok");
        let loaded = AppState::load(&path).expect("load ok");

        assert_eq!(loaded.startup_mode, StartupMode::NewNote);
        assert_eq!(loaded.last_folder.as_deref(), Some(dir.path()));
        assert_eq!(loaded.last_note.as_deref(), Some("Inbox.md"));
        assert!((loaded.left_pane_split_ratio - 0.42).abs() < 1e-6);
        assert_eq!(loaded.auto_save_debounce_secs, 15);
        assert!(!loaded.default_word_wrap);
        assert_eq!(
            loaded.excluded_file_globs,
            vec!["target/**".to_string(), "**/.obsidian/**".to_string()]
        );
        assert_eq!(
            loaded
                .recent_folders
                .first()
                .map(std::path::PathBuf::as_path),
            Some(dir.path()),
        );
    }

    #[test]
    fn rejects_future_version() {
        let dir = TempDir::new().expect("tempdir");
        let path = state_path(&dir);
        let raw = json!({
            "version": 999,
            "startup_mode": "ReloadLast",
            "default_folder": "/tmp/whatever",
        });
        std::fs::write(&path, serde_json::to_vec_pretty(&raw).expect("serialize"))
            .expect("seed file");

        let result = AppState::load(&path);
        assert!(matches!(
            result,
            Err(AppStateError::FutureVersion {
                found: 999,
                supported: 1,
            })
        ));
    }

    #[test]
    fn record_folder_open_dedups_and_caps() {
        let dir = TempDir::new().expect("tempdir");
        let mut state = AppState::default();
        for index in 0..(RECENT_FOLDERS_CAP + 5) {
            let folder = dir.path().join(format!("f{index}"));
            std::fs::create_dir_all(&folder).expect("mkdir");
            state.record_folder_open(&folder);
        }
        assert_eq!(state.recent_folders.len(), RECENT_FOLDERS_CAP);
        // Re-opening an existing folder hoists it to the front without growing.
        let target = state.recent_folders[3].clone();
        state.record_folder_open(&target);
        assert_eq!(state.recent_folders.first(), Some(&target));
        assert_eq!(state.recent_folders.len(), RECENT_FOLDERS_CAP);
    }

    #[test]
    fn clamps_loaded_out_of_range_values() {
        let dir = TempDir::new().expect("tempdir");
        let path = state_path(&dir);
        let raw = json!({
            "version": 1,
            "startup_mode": "ReloadLast",
            "default_folder": "/tmp/x",
            "left_pane_split_ratio": 5.0_f32,
            "auto_save_debounce_secs": 9999,
            "excluded_file_globs": ["", " node_modules/** ", "node_modules/**"],
        });
        std::fs::write(&path, serde_json::to_vec_pretty(&raw).expect("serialize"))
            .expect("seed file");

        let loaded = AppState::load(&path).expect("load ok");
        assert!(loaded.left_pane_split_ratio <= 0.9);
        assert!(loaded.left_pane_split_ratio >= 0.1);
        assert!(loaded.auto_save_debounce_secs <= 60);
        assert!(loaded.auto_save_debounce_secs >= 1);
        assert_eq!(loaded.excluded_file_globs, vec!["node_modules/**"]);
    }

    #[test]
    fn parse_excluded_file_globs_trims_blanks_and_dedups() {
        let parsed = parse_excluded_file_globs(" .git/** \n\nnode_modules/**\n.git/**\r\n");
        assert_eq!(
            parsed,
            vec![".git/**".to_string(), "node_modules/**".to_string()]
        );
    }
}
