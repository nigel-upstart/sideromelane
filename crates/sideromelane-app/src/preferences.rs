//! Preferences window for app-local settings backed by [`crate::state::AppState`].
//!
//! Renders a single `egui::Window` whose lifecycle is controlled by the
//! caller via the `open` flag passed to [`PreferencesWindow::show`]. Returns
//! `true` when any field was edited so the caller can mark
//! `app_state_dirty`. The window deliberately mutates `AppState` in place
//! rather than holding a draft buffer: edits are inexpensive, the schema is
//! flat, and the debounced save loop coalesces rapid changes.

use std::fmt;
use std::path::Path;

use eframe::egui;

use crate::state::{AppState, StartupMode, parse_excluded_file_globs};

/// Errors returned by [`validate_default_folder`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DefaultFolderError {
    /// The path is the filesystem root (`/`).
    FsRoot,
    /// The path is a restricted system directory or a child of one.
    SystemPath(String),
    /// The path is `~/Library` or a child of it.
    UserLibrary,
}

impl fmt::Display for DefaultFolderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FsRoot => write!(formatter, "the filesystem root is not a valid notes folder"),
            Self::SystemPath(path) => write!(
                formatter,
                "'{path}' is a restricted system path; pick another folder",
            ),
            Self::UserLibrary => {
                write!(formatter, "~/Library is restricted; pick another folder")
            }
        }
    }
}

impl std::error::Error for DefaultFolderError {}

/// System root paths whose subtrees must not be used as the default folder.
#[rustfmt::skip]
const RESTRICTED_SYSTEM_PATHS: &[&str] = &[
    "/System", "/private", "/Library",
    "/usr",    "/bin",     "/sbin",
    "/etc",    "/var",     "/dev",
];

/// Validates that `path` is safe to use as the default notes folder.
///
/// Rejects:
/// - The filesystem root `/`
/// - `/System`, `/private`, `/Library`, `/usr`, `/bin`, `/sbin`, `/etc`,
///   `/var`, `/dev` and any subdirectory thereof
/// - `~/Library` and any subdirectory thereof (skipped if the home directory
///   cannot be determined)
///
/// # Errors
///
/// Returns [`DefaultFolderError`] describing the first rejection that applies.
pub fn validate_default_folder(path: &Path) -> Result<(), DefaultFolderError> {
    // Reject the filesystem root.
    if path.components().count() == 1
        && path.components().next() == Some(std::path::Component::RootDir)
    {
        return Err(DefaultFolderError::FsRoot);
    }

    // Reject restricted system paths and their subtrees.
    for &system_path in RESTRICTED_SYSTEM_PATHS {
        let system = Path::new(system_path);
        if path == system || path.starts_with(system) {
            return Err(DefaultFolderError::SystemPath(path.display().to_string()));
        }
    }

    // Reject ~/Library and its subtrees. Skip if home dir is unavailable.
    if let Some(home) = dirs::home_dir() {
        let user_library = home.join("Library");
        if path == user_library || path.starts_with(&user_library) {
            return Err(DefaultFolderError::UserLibrary);
        }
    }

    Ok(())
}

/// State for the Preferences window.
#[derive(Debug, Default)]
pub struct PreferencesWindow {
    /// Transient rejection message to display beneath the folder picker.
    folder_error: Option<String>,
}

impl PreferencesWindow {
    /// Render the window. `open` is toggled to `false` when the user dismisses
    /// it via the close button; the caller owns visibility. Returns `true` if
    /// any field was changed during this frame.
    #[allow(clippy::unused_self, clippy::needless_pass_by_ref_mut)]
    pub fn show(&mut self, ctx: &egui::Context, open: &mut bool, state: &mut AppState) -> bool {
        let mut changed = false;
        let mut window_open = *open;

        egui::Window::new("Preferences")
            .open(&mut window_open)
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.heading("Startup");
                changed |= ui
                    .radio_value(
                        &mut state.startup_mode,
                        StartupMode::ReloadLast,
                        "Reload last folder and note",
                    )
                    .changed();
                changed |= ui
                    .radio_value(
                        &mut state.startup_mode,
                        StartupMode::NewNote,
                        "Open a new note in the default folder",
                    )
                    .changed();

                ui.separator();
                ui.heading("Default folder");
                ui.horizontal(|ui| {
                    ui.label(state.default_folder.display().to_string());
                    if ui.button("Choose\u{2026}").clicked()
                        && let Some(folder) = rfd::FileDialog::new().pick_folder()
                        && folder != state.default_folder
                    {
                        match validate_default_folder(&folder) {
                            Ok(()) => {
                                state.default_folder = folder;
                                self.folder_error = None;
                                changed = true;
                            }
                            Err(error) => {
                                // Keep the previous value; surface the error
                                // in the status area below the picker.
                                self.folder_error = Some(format!(
                                    "Folder '{}' is restricted; pick another.",
                                    folder.display()
                                ));
                                let _ = error;
                            }
                        }
                    }
                });
                if let Some(msg) = &self.folder_error {
                    ui.colored_label(egui::Color32::RED, msg);
                }

                ui.separator();
                ui.heading("Auto-save");
                ui.horizontal(|ui| {
                    let mut secs = state.auto_save_debounce_secs;
                    let response = ui.add(
                        egui::Slider::new(&mut secs, 1..=60)
                            .text("debounce (seconds)")
                            .clamping(egui::SliderClamping::Always),
                    );
                    if response.changed() {
                        state.set_auto_save_debounce_secs(secs);
                        changed = true;
                    }
                });

                ui.separator();
                ui.heading("Editor defaults");
                let mut wrap = state.default_word_wrap;
                if ui
                    .checkbox(&mut wrap, "Word-wrap by default in new folders")
                    .changed()
                {
                    state.default_word_wrap = wrap;
                    changed = true;
                }

                ui.separator();
                ui.heading("Excluded Files");
                let mut excluded_files = state.excluded_file_globs.join("\n");
                let response = ui.add_sized(
                    egui::Vec2::new(ui.available_width(), 96.0),
                    egui::TextEdit::multiline(&mut excluded_files).desired_rows(5),
                );
                if response.changed() {
                    state.excluded_file_globs = parse_excluded_file_globs(&excluded_files);
                    changed = true;
                }
            });

        *open = window_open;
        changed
    }
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use std::path::Path;

    use super::{DefaultFolderError, validate_default_folder};

    #[test]
    fn accepts_normal_user_path() {
        assert!(validate_default_folder(Path::new("/Users/alice/Notes")).is_ok());
    }

    #[test]
    fn rejects_fs_root() {
        let result = validate_default_folder(Path::new("/"));
        assert_eq!(result, Err(DefaultFolderError::FsRoot));
    }

    #[test]
    fn rejects_system_exact() {
        #[rustfmt::skip]
        let paths = [
            "/System", "/private", "/Library",
            "/usr",    "/bin",     "/sbin",
            "/etc",    "/var",     "/dev",
        ];
        for path in &paths {
            let result = validate_default_folder(Path::new(path));
            assert!(
                matches!(result, Err(DefaultFolderError::SystemPath(_))),
                "expected SystemPath rejection for {path}",
            );
        }
    }

    #[test]
    fn rejects_system_child() {
        let result = validate_default_folder(Path::new("/etc/cron.d"));
        assert!(
            matches!(result, Err(DefaultFolderError::SystemPath(_))),
            "expected SystemPath rejection for /etc/cron.d",
        );
    }

    #[test]
    fn rejects_private_child() {
        let result = validate_default_folder(Path::new("/private/tmp/foo"));
        assert!(
            matches!(result, Err(DefaultFolderError::SystemPath(_))),
            "expected SystemPath rejection for /private/tmp/foo",
        );
    }

    #[test]
    fn rejects_user_library() {
        // Only run this check when we can determine a home directory.
        if let Some(home) = dirs::home_dir() {
            let user_library = home.join("Library");
            let result = validate_default_folder(&user_library);
            assert_eq!(result, Err(DefaultFolderError::UserLibrary));
        }
    }

    #[test]
    fn rejects_user_library_child() {
        if let Some(home) = dirs::home_dir() {
            let mail = home.join("Library").join("Mail");
            let result = validate_default_folder(&mail);
            assert_eq!(result, Err(DefaultFolderError::UserLibrary));
        }
    }
}
