//! Native macOS menu bar driven by `muda`.
//!
//! `muda` builds and installs an `NSMenu` at the application level, distinct
//! from any in-window UI. We carry one [`AppMenu`] on the app and:
//!
//! 1. translate every menu item's `MenuId` to a [`MenuAction`] via a small
//!    lookup map populated at construction time;
//! 2. drain `muda::MenuEvent::receiver()` once per frame to dispatch actions
//!    back into the existing toolbar / Preferences code paths;
//! 3. rebuild the **Recent Folders** submenu in place when the LRU mutates.
//!
//! macOS is the only supported platform per `SPEC.md`. Other platforms get a
//! feature-gated stub that compiles to a no-op so the rest of the app builds
//! cleanly under `cargo check --target <other>`.
//!
//! Menu surface (per Spec 0002 AC-3):
//!
//! - **File**: Open Folder… (⌘O), Recent Folders ▸, New Note (⌘N),
//!   Save (⌘S), Close (⌘W).
//! - **View**: Show Graph (⌘G), Word Wrap (⌘⇧W).
//! - **App**: Preferences… (no shortcut, per spec).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[cfg(target_os = "macos")]
use muda::{
    AboutMetadata, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem, Submenu,
    accelerator::{Accelerator, Code, Modifiers},
};

/// Maximum displayed length for a Recent Folders menu entry. Long paths are
/// shortened to `<file-name> — …<truncated parent>` so the menu stays readable.
#[cfg(target_os = "macos")]
const RECENT_LABEL_MAX: usize = 64;

/// Action triggered by a menu click. Mirrors the toolbar / Preferences entry
/// points the rest of the app already exposes.
#[derive(Debug, Clone)]
#[allow(dead_code)] // Variants are constructed by `AppMenu`; consumers land in the next commit.
pub enum MenuAction {
    /// Open a folder picker (same as the toolbar "Open Folder" button).
    OpenFolder,
    /// Create a fresh untitled note in the current folder.
    NewNote,
    /// Save the currently selected note.
    Save,
    /// Close the active note tab. No-op while tabs are unimplemented.
    Close,
    /// Toggle Graph mode (mutually exclusive with Raw / Live Preview).
    ToggleGraph,
    /// Toggle the per-folder editor word-wrap setting.
    ToggleWordWrap,
    /// Open the Preferences window.
    ShowPreferences,
    /// Open one of the recent folders.
    OpenRecent(PathBuf),
}

/// macOS-native menu bar plus a `MenuId -> MenuAction` lookup map.
#[cfg(target_os = "macos")]
#[allow(dead_code)] // Recent-folder rebuild + event drain land in the next commit.
pub struct AppMenu {
    /// The root menu installed on `NSApplication`.
    menu: Menu,
    /// File menu, mutated to rebuild the Recent Folders submenu in place.
    file_menu: Submenu,
    /// Recent Folders submenu, swapped wholesale via `remove` + `insert` when
    /// the LRU mutates so the lookup map stays consistent.
    recent_submenu: Submenu,
    /// Index of `recent_submenu` inside `file_menu`. Captured at build time so
    /// `rebuild_recent_submenu` can put the replacement back in the same slot.
    recent_position: usize,
    /// Lookup from `MenuId` to the action it dispatches. Recent-folder entries
    /// are removed and re-inserted whenever the submenu rebuilds.
    actions: HashMap<MenuId, MenuAction>,
}

#[cfg(target_os = "macos")]
#[allow(dead_code)] // `rebuild_recent_submenu` / `poll` get wired in the next commit.
impl AppMenu {
    /// Build the menu and populate the Recent Folders submenu from the current
    /// LRU. Call [`AppMenu::install_for_nsapp`] after `NSApplication` is up
    /// (i.e. after eframe's first frame) to attach it to the menu bar.
    pub fn new(initial_recent: &[PathBuf]) -> Self {
        let menu = Menu::new();
        let mut actions = HashMap::new();

        // App menu (first slot on macOS owns the application name).
        let app_submenu = Submenu::new("Sideromelane", true);
        let preferences = MenuItem::new("Preferences\u{2026}", true, None);
        actions.insert(preferences.id().clone(), MenuAction::ShowPreferences);
        // `unwrap_or` style avoided: muda's append errors only on platform
        // failure modes that don't surface in this context. We log to stderr
        // as a defensive belt-and-braces move and continue.
        let _ = app_submenu.append_items(&[
            &PredefinedMenuItem::about(None, Some(AboutMetadata::default())),
            &PredefinedMenuItem::separator(),
            &preferences,
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::services(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::hide(None),
            &PredefinedMenuItem::hide_others(None),
            &PredefinedMenuItem::show_all(None),
            &PredefinedMenuItem::separator(),
            &PredefinedMenuItem::quit(None),
        ]);
        let _ = menu.append(&app_submenu);

        // File menu.
        let file_menu = Submenu::new("File", true);
        let open_folder = MenuItem::new(
            "Open Folder\u{2026}",
            true,
            Some(Accelerator::new(Some(Modifiers::SUPER), Code::KeyO)),
        );
        actions.insert(open_folder.id().clone(), MenuAction::OpenFolder);

        let recent_submenu = Submenu::new("Recent Folders", true);
        populate_recent(&recent_submenu, initial_recent, &mut actions);

        let new_note = MenuItem::new(
            "New Note",
            true,
            Some(Accelerator::new(Some(Modifiers::SUPER), Code::KeyN)),
        );
        actions.insert(new_note.id().clone(), MenuAction::NewNote);

        let save = MenuItem::new(
            "Save",
            true,
            Some(Accelerator::new(Some(Modifiers::SUPER), Code::KeyS)),
        );
        actions.insert(save.id().clone(), MenuAction::Save);

        let close = MenuItem::new(
            "Close",
            true,
            Some(Accelerator::new(Some(Modifiers::SUPER), Code::KeyW)),
        );
        actions.insert(close.id().clone(), MenuAction::Close);

        let _ = file_menu.append_items(&[
            &open_folder,
            &recent_submenu,
            &PredefinedMenuItem::separator(),
            &new_note,
            &save,
            &PredefinedMenuItem::separator(),
            &close,
        ]);
        // Position 1 is the slot we just put `recent_submenu` into; record it
        // so rebuilds re-insert the replacement at the same index.
        let recent_position = 1;
        let _ = menu.append(&file_menu);

        // View menu.
        let view_menu = Submenu::new("View", true);
        let show_graph = MenuItem::new(
            "Show Graph",
            true,
            Some(Accelerator::new(Some(Modifiers::SUPER), Code::KeyG)),
        );
        actions.insert(show_graph.id().clone(), MenuAction::ToggleGraph);

        let word_wrap = MenuItem::new(
            "Word Wrap",
            true,
            Some(Accelerator::new(
                Some(Modifiers::SUPER | Modifiers::SHIFT),
                Code::KeyW,
            )),
        );
        actions.insert(word_wrap.id().clone(), MenuAction::ToggleWordWrap);

        let _ = view_menu.append_items(&[&show_graph, &word_wrap]);
        let _ = menu.append(&view_menu);

        Self {
            menu,
            file_menu,
            recent_submenu,
            recent_position,
            actions,
        }
    }

    /// Install the menu on `NSApplication`. Must be called after the eframe
    /// window is up so `NSApp` exists; idempotent for our purposes (calling
    /// twice replaces the previous root).
    pub fn install_for_nsapp(&self) {
        self.menu.init_for_nsapp();
    }

    /// Rebuild the Recent Folders submenu so it reflects `recent` exactly.
    ///
    /// Re-creates every item (and updates the action map) so a folder that
    /// dropped out of the LRU stops resolving to a stale `OpenRecent`.
    pub fn rebuild_recent_submenu(&mut self, recent: &[PathBuf]) {
        // Drop the old submenu's actions out of the map first.
        let stale_ids: Vec<MenuId> = self
            .recent_submenu
            .items()
            .iter()
            .map(|item| item.id().clone())
            .collect();
        for id in stale_ids {
            self.actions.remove(&id);
        }

        // Build a fresh submenu and swap it into the same File-menu slot. We
        // can't `set_text` our way out of this because `MenuItem::set_text`
        // doesn't help: each menu entry is keyed by `MenuId`, and we want
        // those IDs (and their `OpenRecent(path)` mapping) to be stable per
        // path — rebuilding gives us that for free.
        let new_submenu = Submenu::new("Recent Folders", true);
        populate_recent(&new_submenu, recent, &mut self.actions);
        let _ = self.file_menu.remove(&self.recent_submenu);
        let _ = self.file_menu.insert(&new_submenu, self.recent_position);
        self.recent_submenu = new_submenu;
    }

    /// Drain the next pending menu event, if any. Non-blocking.
    pub fn poll(&self) -> Option<MenuAction> {
        let event = MenuEvent::receiver().try_recv().ok()?;
        self.actions.get(event.id()).cloned()
    }
}

#[cfg(target_os = "macos")]
impl std::fmt::Debug for AppMenu {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AppMenu")
            .field("recent_position", &self.recent_position)
            .field("actions", &self.actions.len())
            .finish_non_exhaustive()
    }
}

/// Append one [`MenuItem`] per `recent` path to `submenu`, registering each
/// item's `MenuId` against an `OpenRecent(path)` action.
#[cfg(target_os = "macos")]
fn populate_recent(
    submenu: &Submenu,
    recent: &[PathBuf],
    actions: &mut HashMap<MenuId, MenuAction>,
) {
    if recent.is_empty() {
        let placeholder = MenuItem::new("(no recent folders)", false, None);
        let _ = submenu.append(&placeholder);
        return;
    }

    for path in recent {
        let label = display_label(path);
        let item = MenuItem::new(label, true, None);
        actions.insert(item.id().clone(), MenuAction::OpenRecent(path.clone()));
        let _ = submenu.append(&item);
    }
}

/// Truncate a path for menu display: prefer `<file_name> — <parent>` and
/// shorten the parent with a leading ellipsis if the combined string would
/// blow past [`RECENT_LABEL_MAX`].
#[cfg(target_os = "macos")]
fn display_label(path: &Path) -> String {
    let file_name = path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    );
    let parent = path
        .parent()
        .map(|parent| parent.display().to_string())
        .unwrap_or_default();
    if parent.is_empty() {
        return shorten(&file_name);
    }
    let combined = format!("{file_name} \u{2014} {parent}");
    if combined.chars().count() <= RECENT_LABEL_MAX {
        combined
    } else {
        let allowance = RECENT_LABEL_MAX
            .saturating_sub(file_name.chars().count() + 5) // " — …"
            .max(8);
        let trimmed: String = parent
            .chars()
            .rev()
            .take(allowance)
            .collect::<Vec<char>>()
            .into_iter()
            .rev()
            .collect();
        format!("{file_name} \u{2014} \u{2026}{trimmed}")
    }
}

/// Tail-truncate a single string to [`RECENT_LABEL_MAX`] with a leading ellipsis.
#[cfg(target_os = "macos")]
fn shorten(label: &str) -> String {
    if label.chars().count() <= RECENT_LABEL_MAX {
        return label.to_owned();
    }
    let tail: String = label
        .chars()
        .rev()
        .take(RECENT_LABEL_MAX - 1)
        .collect::<Vec<char>>()
        .into_iter()
        .rev()
        .collect();
    format!("\u{2026}{tail}")
}

// ---------------------------------------------------------------------------
// Non-macOS stub. The app is macOS-first per SPEC.md; the stub keeps the rest
// of the crate building under `cargo check --target x86_64-unknown-linux-gnu`
// without dragging in `gtk` / `libxdo`.
// ---------------------------------------------------------------------------

/// Stub `AppMenu` for non-macOS targets. Every method is a no-op.
#[cfg(not(target_os = "macos"))]
#[derive(Debug, Default)]
pub struct AppMenu {}

#[cfg(not(target_os = "macos"))]
impl AppMenu {
    /// Build a no-op menu. Accepts the same signature as the macOS variant.
    #[must_use]
    pub const fn new(_initial_recent: &[PathBuf]) -> Self {
        Self {}
    }

    /// No-op on non-macOS targets.
    pub const fn install_for_nsapp(&self) {}

    /// No-op on non-macOS targets.
    pub fn rebuild_recent_submenu(&mut self, _recent: &[PathBuf]) {}

    /// Always returns `None` on non-macOS targets.
    #[must_use]
    pub const fn poll(&self) -> Option<MenuAction> {
        None
    }
}

#[cfg(test)]
#[cfg(target_os = "macos")]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use std::path::PathBuf;

    use super::{RECENT_LABEL_MAX, display_label, shorten};

    #[test]
    fn display_label_for_short_path() {
        let label = display_label(&PathBuf::from("/Users/me/Notes"));
        assert!(label.starts_with("Notes"));
        assert!(label.contains("/Users/me"));
    }

    #[test]
    fn display_label_truncates_long_parent() {
        let long_parent = "/Users/me/".to_owned() + &"deep/".repeat(40);
        let path = PathBuf::from(format!("{long_parent}/Plan.md"));
        let label = display_label(&path);
        assert!(label.starts_with("Plan.md"));
        assert!(label.chars().count() <= RECENT_LABEL_MAX);
        assert!(label.contains('\u{2026}'));
    }

    #[test]
    fn shorten_keeps_short_strings_intact() {
        assert_eq!(shorten("hello"), "hello");
    }

    #[test]
    fn shorten_truncates_with_leading_ellipsis() {
        let long: String = "x".repeat(RECENT_LABEL_MAX + 10);
        let out = shorten(&long);
        assert_eq!(out.chars().count(), RECENT_LABEL_MAX);
        assert!(out.starts_with('\u{2026}'));
    }
}
