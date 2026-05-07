//! Preferences window for app-local settings backed by [`crate::state::AppState`].
//!
//! Renders a single `egui::Window` whose lifecycle is controlled by the
//! caller via the `open` flag passed to [`PreferencesWindow::show`]. Returns
//! `true` when any field was edited so the caller can mark
//! `app_state_dirty`. The window deliberately mutates `AppState` in place
//! rather than holding a draft buffer: edits are inexpensive, the schema is
//! flat, and the debounced save loop coalesces rapid changes.

use eframe::egui;

use crate::state::{AppState, StartupMode};

/// State for the Preferences window.
#[derive(Debug, Default)]
pub struct PreferencesWindow {}

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
                        state.default_folder = folder;
                        changed = true;
                    }
                });

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
            });

        *open = window_open;
        changed
    }
}
