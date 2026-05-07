#![allow(missing_docs, clippy::too_many_lines)]

mod autosave;
mod conflict;
mod editor;
mod graph_view;
mod indexer;
mod io;
mod menu;
mod outline;
mod preferences;
mod preview;
mod state;
mod tree;
mod watcher;

use std::collections::HashMap;
use std::fs::{self, File};
use std::io as std_io;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui;
use egui_commonmark::CommonMarkCache;
use sideromelane_core::{
    FolderIndex, FolderSettings, HybridSearchIndex, MarkdownNote, NoteAnalysis, NoteId,
    SearchQuery, WalkOptions, sanitize_asset_filename, validate_image_magic_bytes,
};

use crate::autosave::{AutoSaveOutcome, SELF_WRITE_SUPPRESS_WINDOW, auto_save_dirty_notes};
use crate::conflict::{
    MAX_PENDING_CONFLICTS, WatchOutcome, apply_reload, canonicalize_path, classify_watch_event,
};
use crate::editor::{BlockPreviewCache, live_preview_editor, raw_editor};
use crate::indexer::{Indexer, IndexerCommand, IndexerEvent};
use crate::io::safe_write;
use crate::menu::{AppMenu, MenuAction};
use crate::preferences::{PreferencesWindow, validate_default_folder};
use crate::state::{AppState, StartupMode};

/// Maximum byte size of an image that can be dropped into the assets folder.
const MAX_IMAGE_BYTES: u64 = 32 * 1024 * 1024;
/// Number of bytes inspected when validating image magic bytes.
const IMAGE_HEADER_PEEK: u64 = 16;
/// Height in pixels of the drag handle between the Files and Search sections.
const HANDLE_PX: f32 = 4.0;
/// Maximum number of indexer events to drain per frame. Bounded so background bursts
/// cannot starve UI input handling.
const MAX_INDEXER_EVENTS_PER_FRAME: usize = 16;
/// Debounce window for persisting [`AppState`]. Coalesces rapid edits (slider
/// drags, repeated folder opens) into a single atomic save without blocking
/// the UI on every keystroke.
const APP_STATE_SAVE_DEBOUNCE: Duration = Duration::from_millis(250);
/// Soft size budget for [`CommonMarkCache`]. The cache itself does not expose
/// a usage metric, so we track an approximate byte total ourselves: the sum
/// of `MarkdownBlock::text.len()` for every block fed through the live
/// preview since the last reset. The heuristic is approximate; it assumes
/// `egui_commonmark`'s internal cost (image bitmaps, syntax-highlight
/// state, link hooks) scales roughly linearly with input bytes. Once the
/// running total exceeds this budget, we drop the cache wholesale and start
/// over so memory does not grow unbounded across long sessions.
const COMMONMARK_CACHE_BUDGET_BYTES: usize = 300 * 1024 * 1024;

fn main() -> eframe::Result {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Sideromelane")
            .with_inner_size([1200.0, 780.0]),
        ..Default::default()
    };

    let app_state = AppState::load_or_default();

    eframe::run_native(
        "Sideromelane",
        native_options,
        Box::new(move |_creation_context| Ok(Box::new(SideromelaneApp::new(app_state)))),
    )
}

#[derive(Debug)]
struct SideromelaneApp {
    folder: Option<FolderState>,
    mode: EditorMode,
    search_text: String,
    active_block_index: Option<usize>,
    /// Byte offset to jump to on the next frame. Set by outline row clicks;
    /// consumed by `raw_editor` / `live_preview_editor`.
    pending_jump: Option<usize>,
    status: String,
    indexer: Option<Indexer>,
    graph_view: graph_view::GraphViewState,
    /// Independent graph focus (note or tag). `None` means the graph tracks
    /// the currently selected note. Set when the user clicks a tag node.
    graph_focus: Option<sideromelane_core::GraphNode>,
    app_state: AppState,
    app_state_dirty: bool,
    last_state_save: Instant,
    preferences_open: bool,
    preferences_window: PreferencesWindow,
    /// Set to `true` until the first frame so the app can run its
    /// startup-mode hand-off (open last folder + last note, or initialize
    /// the default folder with a fresh untitled note) inside `update`,
    /// where the egui context is available for any future interactive bits.
    startup_pending: bool,
    /// Caches `egui_commonmark` rendering state (image fetches, syntax-highlighting state,
    /// and the link-hooks registry) across frames so per-block renders are stable.
    commonmark_cache: CommonMarkCache,
    /// Approximate byte total fed into `commonmark_cache` since the last
    /// reset. Reset to zero whenever the cache itself is replaced (folder
    /// switch or budget overrun).
    commonmark_cache_bytes: usize,
    /// Per-block memo for the live-preview pre-pass. Avoids re-running
    /// `transform_wiki_links` and the link-target scan on every frame for
    /// blocks whose `(range, hash)` key has not changed.
    block_preview_cache: BlockPreviewCache,
    /// Set when the user clicks a `sideromelane://note/<NAME>` link in a rendered block.
    /// Drained by `main_panel` after render to navigate to the target note.
    pending_link_click: Option<String>,
    /// Native macOS menu bar. Initialized lazily on the first frame because
    /// `muda` needs `NSApplication` to be running before
    /// `Menu::init_for_nsapp` can attach.
    app_menu: Option<AppMenu>,
    /// Records the wall-clock instant of the last self-initiated `safe_write`
    /// per absolute path. Watcher events arriving inside
    /// [`SELF_WRITE_SUPPRESS_WINDOW`] of one of these timestamps are dropped
    /// so auto-save's own writes do not trigger external-change conflict
    /// detection. See ADR 0013.
    last_self_write_at: HashMap<PathBuf, Instant>,
    /// Filesystem watcher for the currently-open folder. Replaced on every
    /// `open_folder` so the previous folder's notify thread is dropped.
    watcher: Option<watcher::Watcher>,
    /// Notes whose on-disk version diverged while the in-memory buffer was
    /// dirty. Each entry drives a non-blocking conflict modal until the user
    /// resolves it (Reload from disk / Keep mine).
    pending_conflicts: Vec<NoteId>,
    /// Count of conflict events dropped because `pending_conflicts` was at
    /// `MAX_PENDING_CONFLICTS` when they arrived. Surfaced in
    /// `render_conflict_modals` as a single overflow indicator and reset
    /// once the user clears every open modal — subsequent watcher events
    /// then refill `pending_conflicts` normally.
    pending_conflicts_dropped: usize,
}

impl SideromelaneApp {
    fn new(app_state: AppState) -> Self {
        Self {
            folder: None,
            mode: EditorMode::default(),
            search_text: String::new(),
            active_block_index: None,
            pending_jump: None,
            status: String::new(),
            indexer: None,
            graph_view: graph_view::GraphViewState::default(),
            graph_focus: None,
            app_state,
            app_state_dirty: false,
            last_state_save: Instant::now(),
            preferences_open: false,
            preferences_window: PreferencesWindow::default(),
            startup_pending: true,
            commonmark_cache: CommonMarkCache::default(),
            commonmark_cache_bytes: 0,
            block_preview_cache: BlockPreviewCache::new(),
            pending_link_click: None,
            app_menu: None,
            last_self_write_at: HashMap::new(),
            watcher: None,
            pending_conflicts: Vec::new(),
            pending_conflicts_dropped: 0,
        }
    }
}

impl eframe::App for SideromelaneApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.indexer.is_none() {
            self.indexer = Some(Indexer::new(ui.ctx().clone()));
        }
        if self.app_menu.is_none() {
            // First frame: NSApplication is up, attach the menu now.
            let menu = AppMenu::new(&self.app_state.recent_folders);
            menu.install_for_nsapp();
            self.app_menu = Some(menu);
        }
        if self.startup_pending {
            self.startup_pending = false;
            self.run_startup(ui.ctx());
        }
        self.drain_menu_events(ui.ctx());
        self.drain_indexer_events();
        self.drain_watcher_events();
        self.handle_dropped_files(ui.ctx());
        self.auto_save_tick();

        egui::Panel::top("top_bar").show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Open Folder").clicked() {
                    self.pick_folder(ui.ctx());
                }
                let can_use_folder = self.folder.is_some();
                if ui
                    .add_enabled(can_use_folder, egui::Button::new("New"))
                    .clicked()
                {
                    self.new_note();
                }
                if ui
                    .add_enabled(can_use_folder, egui::Button::new("Save"))
                    .clicked()
                {
                    self.save_selected();
                }
                ui.separator();
                ui.selectable_value(&mut self.mode, EditorMode::Raw, "Raw");
                ui.selectable_value(&mut self.mode, EditorMode::LivePreview, "Live Preview");
                ui.selectable_value(&mut self.mode, EditorMode::Graph, "Graph");
                ui.separator();
                if ui.button("Preferences\u{2026}").clicked() {
                    self.preferences_open = !self.preferences_open;
                }
                ui.separator();
                ui.label(&self.status);
            });
        });

        egui::Panel::left("left_panel")
            .resizable(true)
            .default_size(260.0)
            .show_inside(ui, |ui| self.left_panel(ui));

        egui::Panel::right("right_panel")
            .resizable(true)
            .default_size(300.0)
            .show_inside(ui, |ui| self.right_panel(ui));

        egui::CentralPanel::default().show_inside(ui, |ui| self.main_panel(ui));

        self.render_conflict_modals(ui.ctx());

        if self.preferences_open {
            let context = ui.ctx().clone();
            let changed = self.preferences_window.show(
                &context,
                &mut self.preferences_open,
                &mut self.app_state,
            );
            if changed {
                self.app_state_dirty = true;
            }
        }

        // Track the last selected note in `last_note` so the next launch can
        // reopen exactly where the user left off. The folder-side bookkeeping
        // (last_folder, recent_folders) lives in `open_folder`.
        if let Some(folder) = self.folder.as_ref()
            && let Some(note) = folder.selected_note()
        {
            let relative = note.note_id.relative_path().display().to_string();
            if self.app_state.last_note.as_deref() != Some(&relative) {
                self.app_state.last_note = Some(relative);
                self.app_state_dirty = true;
            }
        }

        self.maybe_persist_app_state();
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum EditorMode {
    #[default]
    Raw,
    LivePreview,
    Graph,
}

/// User choice when a watcher reports an external change to a dirty note.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConflictChoice {
    /// Replace the in-memory buffer with the on-disk version. Discards
    /// unsaved edits.
    Reload,
    /// Keep the in-memory buffer. The next auto-save sweep overwrites the
    /// disk version.
    Keep,
}

#[derive(Debug)]
struct FolderState {
    root: PathBuf,
    notes: Vec<NoteRecord>,
    selected: Option<usize>,
    search_index: HybridSearchIndex,
    folder_index: FolderIndex,
    settings: FolderSettings,
    /// `true` once the indexer has published its first `IndexUpdated` event
    /// for the current folder. Until then the search/backlinks/graph panels
    /// surface a placeholder rather than a misleading empty state.
    indexes_ready: bool,
    /// Cached file-tree rendering view. Rebuilt lazily in `left_panel` and
    /// invalidated whenever the underlying note set changes (indexer events,
    /// `new_note`, image embed inserts).
    cached_tree: Option<tree::Tree>,
    /// Transient set of directory paths auto-expanded so the selected note is
    /// visible. Kept in memory only (never persisted) so reopening the folder
    /// preserves the user's explicit expand/collapse choices, and dedup'd via
    /// the set rather than appended every frame.
    auto_expanded: std::collections::BTreeSet<String>,
    /// Maps each note's absolute path to its index in `notes`. Refreshed
    /// whenever `notes` is mutated (`merge_discovered_notes`, `new_note`).
    /// Lets `classify_watch_event` resolve a watcher event in O(1) rather
    /// than scanning the full note set per event — important when a `git
    /// pull` produces a burst spanning hundreds of files.
    note_path_index: HashMap<PathBuf, usize>,
}

impl FolderState {
    /// Quick-open the folder without performing any indexing work.
    ///
    /// Discovery is shallow: we load per-folder settings and read at most one note
    /// (`Untitled.md` if present, otherwise the first Markdown file we find) so the
    /// user has an editable surface immediately. The full rescan is dispatched to
    /// the background indexer by the caller, and the search/folder indexes stay
    /// empty (showing "Indexing…") until the first `IndexUpdated` event arrives.
    fn load(root: PathBuf) -> std_io::Result<Self> {
        let settings = FolderSettings::load(&root).map_err(std_io::Error::other)?;
        let initial = initial_note(&root)?;
        let (notes, selected) =
            initial.map_or_else(|| (Vec::new(), None), |record| (vec![record], Some(0)));

        let note_path_index = build_note_path_index(&notes);
        Ok(Self {
            root,
            notes,
            selected,
            search_index: HybridSearchIndex::default(),
            folder_index: FolderIndex::default(),
            settings,
            indexes_ready: false,
            cached_tree: None,
            auto_expanded: std::collections::BTreeSet::new(),
            note_path_index,
        })
    }

    /// Rebuild `note_path_index` from `notes`. Call after any mutation that
    /// changes the set or order of notes.
    fn rebuild_note_path_index(&mut self) {
        self.note_path_index = build_note_path_index(&self.notes);
    }

    fn selected_note(&self) -> Option<&NoteRecord> {
        self.selected.and_then(|index| self.notes.get(index))
    }

    fn selected_note_mut(&mut self) -> Option<&mut NoteRecord> {
        self.selected.and_then(|index| self.notes.get_mut(index))
    }

    fn selected_parsed_note(&self) -> Option<MarkdownNote> {
        self.selected_note()
            .map(|note| MarkdownNote::parse(note.note_id.clone(), note.source.clone()))
    }
}

#[derive(Debug)]
struct NoteRecord {
    note_id: NoteId,
    absolute_path: PathBuf,
    source: String,
    dirty: bool,
    /// Last time the user (or a programmatic edit) mutated `source`. Used by
    /// the auto-save tick to skip notes the user is still actively editing.
    last_edit_at: Instant,
}

impl NoteRecord {
    fn read(root: &Path, absolute_path: PathBuf) -> std_io::Result<Self> {
        let relative_path = absolute_path
            .strip_prefix(root)
            .map_err(std_io::Error::other)?
            .to_path_buf();
        let note_id =
            NoteId::from_folder_relative_path(relative_path).map_err(std_io::Error::other)?;
        let source = fs::read_to_string(&absolute_path)?;

        Ok(Self {
            note_id,
            absolute_path,
            source,
            dirty: false,
            last_edit_at: Instant::now(),
        })
    }
}

impl SideromelaneApp {
    fn pick_folder(&mut self, ctx: &egui::Context) {
        let Some(root) = rfd::FileDialog::new().pick_folder() else {
            return;
        };

        self.open_folder(root, ctx);
    }

    fn open_folder(&mut self, root: PathBuf, ctx: &egui::Context) {
        match FolderState::load(root) {
            Ok(folder) => {
                self.status = format!("Opened {}", folder.root.display());
                let scan_root = folder.root.clone();
                self.app_state.record_folder_open(&scan_root);
                // Drop the previous folder's CommonMark cache so cached
                // image fetches, link hooks, and syntax-highlight state do
                // not leak across folders. The per-block preview cache is
                // also reset because its keys are byte ranges into the
                // previous folder's notes.
                self.commonmark_cache = CommonMarkCache::default();
                self.commonmark_cache_bytes = 0;
                self.block_preview_cache.clear();
                // Drop the previous note's `last_note` so the post-`update`
                // bookkeeping picks up whatever this folder selects on the
                // next frame rather than carrying over a stale relative path.
                self.app_state.last_note = None;
                self.app_state_dirty = true;
                self.folder = Some(folder);
                self.active_block_index = None;
                if let Some(menu) = self.app_menu.as_mut() {
                    menu.rebuild_recent_submenu(&self.app_state.recent_folders);
                }
                // Replace the previous folder's watcher (if any). Watcher
                // failures are surfaced in the status bar but never block
                // the open — the app remains usable with auto-save only.
                self.watcher = match watcher::Watcher::new(&scan_root, ctx.clone()) {
                    Ok(watcher) => Some(watcher),
                    Err(error) => {
                        self.status = format!("File watch unavailable: {error}");
                        None
                    }
                };
                self.pending_conflicts.clear();
                self.last_self_write_at.clear();
                self.dispatch_rescan(scan_root);
            }
            Err(error) => self.status = format!("Open failed: {error}"),
        }
    }

    /// Boot-time hand-off driven by `app_state.startup_mode`. Called once
    /// from `update` on the first frame so the eframe context is live in
    /// case any startup branch wants to surface a dialog.
    fn run_startup(&mut self, ctx: &egui::Context) {
        match self.app_state.startup_mode {
            StartupMode::ReloadLast => {
                if let Some(folder) = self.app_state.last_folder.clone()
                    && folder.is_dir()
                {
                    let target_note = self.app_state.last_note.clone();
                    self.open_folder(folder, ctx);
                    if let Some(relative) = target_note
                        && let Some(state) = self.folder.as_mut()
                        && let Some(index) = state.notes.iter().position(|note| {
                            note.note_id.relative_path().to_string_lossy() == relative
                        })
                    {
                        state.selected = Some(index);
                        self.active_block_index = None;
                    }
                    return;
                }
                self.boot_default_folder(ctx);
            }
            StartupMode::NewNote => {
                self.boot_default_folder(ctx);
                self.new_note();
            }
        }
    }

    /// Open (creating if needed) the configured default folder.
    fn boot_default_folder(&mut self, ctx: &egui::Context) {
        let default_folder = self.app_state.default_folder.clone();
        if let Err(error) = validate_default_folder(&default_folder) {
            self.status = format!(
                "Default folder '{}' is restricted; pick another. ({error})",
                default_folder.display()
            );
            return;
        }
        if let Err(error) = fs::create_dir_all(&default_folder) {
            self.status = format!(
                "Default folder unavailable ({}): {error}",
                default_folder.display()
            );
            return;
        }
        self.open_folder(default_folder, ctx);
    }

    /// Persist `app_state` if it has been marked dirty and the debounce
    /// window has elapsed. Called once per frame from `update`. Errors are
    /// surfaced in `status` but never block the UI.
    fn maybe_persist_app_state(&mut self) {
        if !self.app_state_dirty {
            return;
        }
        if self.last_state_save.elapsed() < APP_STATE_SAVE_DEBOUNCE {
            return;
        }
        match self.app_state.save_default() {
            Ok(()) => {
                self.app_state_dirty = false;
                self.last_state_save = Instant::now();
            }
            Err(error) => {
                self.status = format!("App state save failed: {error}");
                // Leave the dirty flag set so a future frame retries; reset
                // the timer so we don't busy-loop on the failure.
                self.last_state_save = Instant::now();
            }
        }
    }

    fn new_note(&mut self) {
        let Some(folder) = self.folder.as_mut() else {
            return;
        };
        let Some((note_id, absolute_path)) = next_untitled_note(&folder.root) else {
            self.status = format!(
                "Couldn't find an unused Untitled.md name (tried {NEXT_UNTITLED_NOTE_MAX_ATTEMPTS})"
            );
            return;
        };
        let source = format!("# {}\n", note_id.file_stem());
        let new_index = folder.notes.len();
        folder
            .note_path_index
            .insert(absolute_path.clone(), new_index);
        folder.notes.push(NoteRecord {
            note_id,
            absolute_path,
            source,
            dirty: true,
            last_edit_at: Instant::now(),
        });
        folder.selected = Some(new_index);
        folder.cached_tree = None;
        let scan_root = folder.root.clone();
        self.active_block_index = None;
        // Adding an unsaved note to disk is deferred to save; still trigger a
        // rescan so the indexer notices any sibling note changes that may
        // have happened externally.
        self.dispatch_rescan(scan_root);
    }

    fn save_selected(&mut self) {
        let Some(folder) = self.folder.as_mut() else {
            return;
        };
        let Some(note) = folder.selected_note_mut() else {
            return;
        };

        match safe_write(&note.absolute_path, &note.source) {
            Ok(()) => {
                note.dirty = false;
                let note_id = note.note_id.clone();
                let source = note.source.clone();
                let absolute_path = note.absolute_path.clone();
                let relative = note.note_id.relative_path().display().to_string();
                self.status = format!("Saved {relative}");
                self.last_self_write_at
                    .insert(canonicalize_path(&absolute_path), Instant::now());
                if let Some(indexer) = self.indexer.as_ref() {
                    indexer.send(IndexerCommand::NoteChanged { note_id, source });
                }
            }
            Err(error) => self.status = format!("Save failed: {error}"),
        }
    }

    /// Drain pending menu events and dispatch each one. Bounded per frame
    /// for the same reason `drain_indexer_events` is bounded — a hostile or
    /// pathological burst must not starve UI input handling.
    fn drain_menu_events(&mut self, ctx: &egui::Context) {
        for _ in 0..MAX_INDEXER_EVENTS_PER_FRAME {
            let Some(action) = self.app_menu.as_ref().and_then(AppMenu::poll) else {
                break;
            };
            self.dispatch_menu_action(action, ctx);
        }
    }

    fn dispatch_menu_action(&mut self, action: MenuAction, ctx: &egui::Context) {
        match action {
            MenuAction::OpenFolder => self.pick_folder(ctx),
            MenuAction::NewNote => self.new_note(),
            MenuAction::Save => self.save_selected(),
            MenuAction::Close => self.close_active_note(),
            MenuAction::ToggleGraph => self.toggle_graph_mode(),
            MenuAction::ToggleWordWrap => self.toggle_word_wrap(),
            MenuAction::ShowPreferences => self.show_preferences(),
            MenuAction::OpenRecent(path) => self.open_folder(path, ctx),
        }
    }

    /// Spec 0002 AC-3: "Close (⌘W) — closes the active note tab (no-op if
    /// not yet implemented)." Tabs aren't here yet; surface the no-op in the
    /// status bar so the shortcut doesn't feel broken.
    fn close_active_note(&mut self) {
        self.status = "Close: tabs not yet implemented".into();
    }

    /// Toggle Graph mode. When already in Graph, drop back to Raw so the
    /// shortcut acts as a true toggle rather than locking the user in.
    /// Clears `graph_focus` on entry so the graph always opens on the current
    /// note rather than a stale tag focus from a prior session.
    fn toggle_graph_mode(&mut self) {
        self.mode = if self.mode == EditorMode::Graph {
            EditorMode::Raw
        } else {
            self.graph_focus = None;
            EditorMode::Graph
        };
    }

    /// Toggle the per-folder editor word-wrap setting and persist it. Mirrors
    /// the right-panel checkbox: the setting is folder-scoped and triggers a
    /// settings save, but does not require a rescan since walker behavior is
    /// unchanged.
    fn toggle_word_wrap(&mut self) {
        let Some(folder) = self.folder.as_mut() else {
            return;
        };
        folder.settings.ui.editor_word_wrap = !folder.settings.ui.editor_word_wrap;
        if let Err(error) = folder.settings.save(&folder.root) {
            self.status = format!("Settings save failed: {error}");
        }
    }

    const fn show_preferences(&mut self) {
        self.preferences_open = true;
    }

    /// Walk dirty notes and persist any whose last edit is older than the
    /// configured auto-save debounce. Errors are surfaced in the status bar
    /// without clearing `dirty`, so the next tick retries.
    fn auto_save_tick(&mut self) {
        let Some(folder) = self.folder.as_mut() else {
            return;
        };
        let debounce =
            Duration::from_secs(u64::from(self.app_state.auto_save_debounce_secs.max(1)));
        let now = Instant::now();
        let sweep = auto_save_dirty_notes(&mut folder.notes, debounce, now);
        for AutoSaveOutcome {
            note_id,
            source,
            absolute_path,
            relative,
        } in sweep.saved
        {
            self.last_self_write_at
                .insert(canonicalize_path(&absolute_path), Instant::now());
            self.status = format!("Auto-saved {relative}");
            if let Some(indexer) = self.indexer.as_ref() {
                indexer.send(IndexerCommand::NoteChanged { note_id, source });
            }
        }
        if let Some((relative, error)) = sweep.first_error {
            self.status = format!("Auto-save failed for {relative}: {error}");
        }

        // Prune stale entries so the suppression map can't grow unbounded
        // over a long session. The 4x factor leaves a comfortable margin
        // past the suppression window (any entry older than that is no
        // longer doing real work) while bounding memory by roughly the
        // save rate over a 4 * SELF_WRITE_SUPPRESS_WINDOW (~800 ms) span.
        self.last_self_write_at
            .retain(|_, stamp| stamp.elapsed() < SELF_WRITE_SUPPRESS_WINDOW * 4);
    }

    fn dispatch_rescan(&mut self, root: PathBuf) {
        let options = self
            .folder
            .as_ref()
            .map(|folder| walk_options_for(&folder.settings))
            .unwrap_or_default();
        if let Some(folder) = self.folder.as_mut() {
            folder.indexes_ready = false;
        }
        if let Some(indexer) = self.indexer.as_ref() {
            indexer.send(IndexerCommand::Rescan { root, options });
        }
    }

    fn drain_indexer_events(&mut self) {
        for _ in 0..MAX_INDEXER_EVENTS_PER_FRAME {
            let Some(event) = self.indexer.as_ref().and_then(Indexer::poll) else {
                break;
            };
            self.apply_indexer_event(event);
        }
    }

    /// Pop every pending watcher event and dispatch each one.
    ///
    /// Events for paths the app is not tracking (e.g. `assets/`, hidden
    /// files, freshly created notes the indexer hasn't surfaced yet) are
    /// silently ignored — the indexer rescan triggered after a save handles
    /// any structural divergence.
    fn drain_watcher_events(&mut self) {
        if self.watcher.is_none() {
            return;
        }
        // Cap per-frame work so a sudden burst (e.g. `git pull` rewriting
        // hundreds of notes) cannot starve UI input handling. Each `Reload`
        // does a synchronous `read_to_string` on the UI thread, so we
        // deliberately prefer responsiveness over draining the whole burst
        // in one frame; remaining events get picked up on subsequent frames.
        for _ in 0..MAX_INDEXER_EVENTS_PER_FRAME {
            let Some(event) = self.watcher.as_ref().and_then(watcher::Watcher::poll) else {
                break;
            };
            self.apply_watch_event(&event);
        }
    }

    fn apply_watch_event(&mut self, event: &watcher::WatchEvent) {
        let Some(folder) = self.folder.as_mut() else {
            return;
        };
        let now = Instant::now();
        match classify_watch_event(
            event,
            &folder.notes,
            &folder.note_path_index,
            &self.last_self_write_at,
            SELF_WRITE_SUPPRESS_WINDOW,
            now,
        ) {
            WatchOutcome::Ignored | WatchOutcome::Suppressed | WatchOutcome::UnknownPath => {}
            WatchOutcome::Conflict(note_id) => {
                if self.pending_conflicts.contains(&note_id) {
                    // Already queued — no work to do.
                } else if self.pending_conflicts.len() >= MAX_PENDING_CONFLICTS {
                    // Cap reached. Track the overflow so the UI can surface a
                    // "+ N more" indicator instead of spawning an unbounded
                    // number of windows. The next batch of events refills
                    // normally once the user clears the open modals.
                    self.pending_conflicts_dropped =
                        self.pending_conflicts_dropped.saturating_add(1);
                } else {
                    self.pending_conflicts.push(note_id);
                }
            }
            WatchOutcome::Reload { index } => {
                let path = folder.notes[index].absolute_path.clone();
                match fs::read_to_string(&path) {
                    Ok(source) => {
                        apply_reload(&mut folder.notes, index, source);
                        // If the reloaded note is currently selected and we
                        // are mid Live-Preview, the cached active block index
                        // may now point past the new block list — drop it so
                        // the renderer re-derives it from the fresh source.
                        if folder.selected.is_some_and(|i| i == index) {
                            self.active_block_index = None;
                        }
                    }
                    Err(error) => {
                        self.status = format!("Reload failed: {error}");
                    }
                }
            }
        }
    }

    fn apply_indexer_event(&mut self, event: IndexerEvent) {
        match event {
            IndexerEvent::ScanFailed { root, message } => {
                self.status = format!("Scan failed for {}: {message}", root.display());
                if let Some(folder) = self.folder.as_mut() {
                    folder.indexes_ready = false;
                }
            }
            IndexerEvent::NotesDiscovered(records) => {
                if let Some(folder) = self.folder.as_mut() {
                    merge_discovered_notes(folder, records);
                    folder.cached_tree = None;
                }
            }
            IndexerEvent::IndexUpdated {
                search,
                folder: folder_index,
            } => {
                if let Some(folder) = self.folder.as_mut() {
                    folder.search_index = search;
                    folder.folder_index = folder_index;
                    folder.indexes_ready = true;
                    folder.cached_tree = None;
                }
            }
        }
    }

    fn handle_dropped_files(&mut self, context: &egui::Context) {
        let dropped_files = context.input(|input| input.raw.dropped_files.clone());
        if dropped_files.is_empty() {
            return;
        }

        for dropped_file in dropped_files {
            let Some(path) = dropped_file.path else {
                continue;
            };
            if is_image_path(&path) {
                self.insert_image_embed(&path);
            }
        }
    }

    fn insert_image_embed(&mut self, source_path: &Path) {
        let Some(folder) = self.folder.as_mut() else {
            return;
        };
        let assets_dir = folder.root.join("assets");
        let Some(file_name) = source_path.file_name().and_then(|name| name.to_str()) else {
            self.status = "Image dropped without a usable filename".into();
            return;
        };

        let safe_name = match sanitize_asset_filename(file_name) {
            Ok(name) => name,
            Err(error) => {
                self.status = format!("Image rejected: {error}");
                return;
            }
        };

        match fs::metadata(source_path) {
            Ok(metadata) if metadata.len() > MAX_IMAGE_BYTES => {
                self.status = "Image too large (max 32 MiB)".into();
                return;
            }
            Ok(_) => {}
            Err(error) => {
                self.status = format!("Image stat failed: {error}");
                return;
            }
        }

        let mut header = [0_u8; 16];
        let header_len = match read_image_header(source_path, &mut header) {
            Ok(len) => len,
            Err(error) => {
                self.status = format!("Image read failed: {error}");
                return;
            }
        };

        if let Err(error) = validate_image_magic_bytes(&header[..header_len]) {
            self.status = format!("Image rejected: {error}");
            return;
        }

        let Some(target_path) = unique_asset_path(&assets_dir, &safe_name) else {
            self.status = "Image rejected: too many name collisions in assets/".into();
            return;
        };

        let Ok(canonical_root) = folder.root.canonicalize() else {
            self.status = "Image rejected: target outside folder".into();
            return;
        };
        let Some(canonical_target) = canonicalize_target(&target_path) else {
            self.status = "Image rejected: target outside folder".into();
            return;
        };
        if !canonical_target.starts_with(&canonical_root) {
            self.status = "Image rejected: target outside folder".into();
            return;
        }
        if let Ok(metadata) = fs::symlink_metadata(&target_path)
            && metadata.file_type().is_symlink()
        {
            self.status = "Image rejected: target is a symlink".into();
            return;
        }

        match copy_asset(source_path, &target_path) {
            Ok(()) => {
                let relative_target = target_path
                    .strip_prefix(&folder.root)
                    .unwrap_or(&target_path)
                    .to_string_lossy()
                    .into_owned();
                let Some(note) = folder.selected_note_mut() else {
                    return;
                };
                note.source.push('\n');
                note.source.push_str("![[");
                note.source.push_str(&relative_target);
                note.source.push_str("]]\n");
                note.dirty = true;
                note.last_edit_at = Instant::now();
                self.status = format!("Inserted {relative_target}");
                folder.cached_tree = None;
                let scan_root = folder.root.clone();
                self.dispatch_rescan(scan_root);
            }
            Err(error) => self.status = format!("Image copy failed: {error}"),
        }
    }

    fn left_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Files");
        let Some(folder) = self.folder.as_mut() else {
            return;
        };

        // Auto-expand ancestors of the selected note into a transient
        // in-memory set so the user lands on a tree where their current note
        // is visible, without mutating (and re-persisting) the explicit
        // `tree_expanded_paths` every frame.
        if let Some(selected_note) = folder
            .selected
            .and_then(|index| folder.notes.get(index))
            .map(|record| record.note_id.clone())
        {
            for ancestor in tree::ancestor_paths(&selected_note) {
                folder.auto_expanded.insert(ancestor);
            }
        }

        let mut selected_note = folder.selected;
        // Temporarily move the cached tree out of `folder` so we can pass
        // `&mut folder` into the render helpers (which need to mutate
        // `folder.settings.ui.tree_expanded_paths`). The cache is restored
        // before the function returns. Build on demand if missing.
        let folder_tree = folder.cached_tree.take().unwrap_or_else(|| {
            let note_ids: Vec<NoteId> = folder
                .notes
                .iter()
                .map(|record| record.note_id.clone())
                .collect();
            tree::build_tree(&note_ids)
        });
        let mut tree_changed = false;

        // --- Resizable splitter between Files and Search ---
        let ratio = self.app_state.left_pane_split_ratio;
        let total_height = ui.available_height();
        let panel_width = ui.available_width();
        let files_height = clamp_split_height(ratio, total_height);

        ui.allocate_ui(egui::Vec2::new(panel_width, files_height), |ui| {
            egui::ScrollArea::vertical()
                .id_salt("files_tree")
                .show(ui, |ui| {
                    for note_id in &folder_tree.root_notes {
                        render_note_row(ui, folder, &mut selected_note, note_id, 0);
                    }
                    for subdir in &folder_tree.subdirs {
                        render_dir(ui, folder, &mut selected_note, subdir, 0, &mut tree_changed);
                    }
                });
        });

        folder.cached_tree = Some(folder_tree);

        if selected_note != folder.selected {
            folder.selected = selected_note;
            self.active_block_index = None;
            self.graph_focus = None;
        }

        let pending_settings_save = if tree_changed {
            Some(folder.root.clone())
        } else {
            None
        };

        // Drag handle
        let handle_rect = ui.allocate_space(egui::Vec2::new(panel_width, HANDLE_PX)).1;
        let handle_response = ui.interact(
            handle_rect,
            ui.id().with("split_handle"),
            egui::Sense::drag(),
        );
        ui.painter()
            .rect_filled(handle_rect, 0.0, egui::Color32::from_gray(60));
        if handle_response.dragged() {
            let new_files_height = (files_height + handle_response.drag_delta().y)
                .clamp(80.0, total_height - 80.0 - HANDLE_PX);
            let new_ratio = new_files_height / total_height;
            self.app_state.set_left_pane_split_ratio(new_ratio);
            self.app_state_dirty = true;
        }
        if handle_response.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
        }

        // Search section occupies the remaining space
        ui.heading("Search");
        ui.add(egui::TextEdit::singleline(&mut self.search_text).hint_text("Search"));
        if !folder.indexes_ready {
            ui.label("Indexing\u{2026}");
        }

        let query = if self.search_text.trim().is_empty() {
            SearchQuery::empty()
        } else {
            SearchQuery::text(self.search_text.clone())
        };
        let results = folder.search_index.search(&query);
        egui::ScrollArea::vertical()
            .id_salt("search_results")
            .show(ui, |ui| {
                for result in results {
                    if let Some(index) = folder
                        .notes
                        .iter()
                        .position(|note| &note.note_id == result.note_id())
                    {
                        let note = &folder.notes[index];
                        if ui
                            .selectable_label(
                                folder.selected == Some(index),
                                format!(
                                    "{} ({:.1})",
                                    note.note_id.file_stem(),
                                    result.combined_score()
                                ),
                            )
                            .clicked()
                        {
                            folder.selected = Some(index);
                            self.active_block_index = None;
                            self.graph_focus = None;
                        }
                    }
                }
            });

        // Persist tree expansion state outside the folder borrow.
        let _ = folder;
        if let Some(root) = pending_settings_save
            && let Some(folder) = self.folder.as_ref()
            && let Err(error) = folder.settings.save(&root)
        {
            self.status = format!("Tree state save failed: {error}");
        }
    }

    fn right_panel(&mut self, ui: &mut egui::Ui) {
        let Some(folder) = self.folder.as_mut() else {
            return;
        };

        let mut walker_changed = false;
        let mut ui_changed = false;
        ui.collapsing("Folder settings", |ui| {
            walker_changed |= ui
                .checkbox(
                    &mut folder.settings.ignore.include_dotfiles,
                    "Include dotfiles",
                )
                .changed();
            walker_changed |= ui
                .checkbox(
                    &mut folder.settings.ignore.honor_gitignore,
                    "Honor .gitignore",
                )
                .changed();
        });
        ui.collapsing("Editor", |ui| {
            ui_changed |= ui
                .checkbox(
                    &mut folder.settings.ui.editor_word_wrap,
                    "Word wrap (off = horizontal scroll)",
                )
                .changed();
        });
        let settings_dirty = walker_changed || ui_changed;
        let pending_rescan_root = if settings_dirty {
            match folder.settings.save(&folder.root) {
                Ok(()) => walker_changed.then(|| folder.root.clone()),
                Err(error) => {
                    self.status = format!("Settings save failed: {error}");
                    None
                }
            }
        } else {
            None
        };

        ui.heading("Backlinks");
        if let Some(note) = folder.selected_note() {
            let backlink_sources = folder
                .folder_index
                .backlinks_to(&note.note_id)
                .iter()
                .map(|backlink| backlink.source().clone())
                .collect::<Vec<_>>();
            for source in backlink_sources {
                if ui.button(source.file_stem()).clicked() {
                    select_note(folder, &source);
                    self.active_block_index = None;
                    self.graph_focus = None;
                }
            }
        }

        ui.separator();
        ui.heading("Outline");
        if let Some(parsed_note) = folder.selected_parsed_note() {
            let source = folder
                .selected_note()
                .map(|n| n.source.clone())
                .unwrap_or_default();
            let analysis = NoteAnalysis::from_note(&parsed_note);
            let base_font = ui
                .style()
                .text_styles
                .get(&egui::TextStyle::Body)
                .map_or(14.0, |id| id.size);
            for heading in analysis.headings() {
                let display = outline::display_heading_text(heading.text());
                if display.is_empty() {
                    continue;
                }
                let level = heading.level();
                let response = ui.horizontal(|ui| {
                    ui.add_space(outline::heading_indent_px(level));
                    let mut rich = egui::RichText::new(&display)
                        .size(outline::heading_font_size(level, base_font));
                    if outline::heading_is_bold(level) {
                        rich = rich.strong();
                    }
                    ui.add(egui::Label::new(rich).sense(egui::Sense::click()))
                });
                if response.inner.clicked()
                    && let Some(offset) = outline::byte_offset_for_heading(&source, level, &display)
                {
                    self.pending_jump = Some(offset);
                }
            }
        }

        // Folder borrow ends with this scope; perform any deferred rescan
        // dispatch now that we can take a fresh `&mut self` borrow.
        let _ = folder;
        if let Some(root) = pending_rescan_root {
            self.status = "Folder settings updated".into();
            self.active_block_index = None;
            self.dispatch_rescan(root);
        }
    }

    fn main_panel(&mut self, ui: &mut egui::Ui) {
        let Some(folder) = self.folder.as_mut() else {
            ui.centered_and_justified(|ui| {
                ui.heading("Sideromelane");
            });
            return;
        };
        let Some(index) = folder.selected else {
            ui.centered_and_justified(|ui| {
                ui.heading("No Notes");
            });
            return;
        };

        ui.horizontal(|ui| {
            let note = &folder.notes[index];
            ui.heading(note.note_id.file_stem());
            if note.dirty {
                ui.label("Unsaved");
            }
        });
        ui.separator();

        let word_wrap = folder.settings.ui.editor_word_wrap;
        let folder_root = folder.root.clone();
        let changed = match self.mode {
            EditorMode::Raw => raw_editor(
                ui,
                &mut folder.notes[index],
                word_wrap,
                &mut self.pending_jump,
            ),
            EditorMode::LivePreview => live_preview_editor(
                ui,
                &mut folder.notes[index],
                &mut self.active_block_index,
                word_wrap,
                &mut self.pending_jump,
                &folder_root,
                &mut self.commonmark_cache,
                &mut self.commonmark_cache_bytes,
                &mut self.block_preview_cache,
                &mut self.pending_link_click,
            ),
            EditorMode::Graph => {
                let default_focus = graph_view::note_focus(&folder.notes[index].note_id);
                let focus = self.graph_focus.as_ref().unwrap_or(&default_focus);
                let clicked = graph_view::draw(
                    ui,
                    &mut self.graph_view,
                    &folder.folder_index,
                    Some(focus),
                    graph_view::DEFAULT_DEPTH,
                );
                match clicked {
                    Some(sideromelane_core::GraphNode::Note { ref note_id })
                        if note_id != &folder.notes[index].note_id =>
                    {
                        select_note(folder, note_id);
                        self.active_block_index = None;
                        self.graph_focus = None;
                    }
                    Some(tag_node @ sideromelane_core::GraphNode::Tag { .. }) => {
                        self.graph_focus = Some(tag_node);
                    }
                    _ => {}
                }
                false
            }
        };

        if changed {
            let note = &mut folder.notes[index];
            note.dirty = true;
            // Stamp the edit so the auto-save tick waits for inactivity
            // before persisting. Index refresh is deferred to save; typing
            // must not block on re-indexing every keystroke.
            note.last_edit_at = Instant::now();
        }

        // Drain any pending in-app link click. Navigates to the target note if it exists.
        if let Some(target) = self.pending_link_click.take() {
            self.navigate_to_note_by_name(&target);
        }

        // Soft-reset the CommonMark cache once we cross the byte budget.
        // We do this AFTER rendering so the visible blocks were drawn from
        // the live cache; the next frame repopulates from cold but with
        // bounded memory. The budget is approximate (see
        // `COMMONMARK_CACHE_BUDGET_BYTES` doc).
        if self.commonmark_cache_bytes > COMMONMARK_CACHE_BUDGET_BYTES {
            self.commonmark_cache = CommonMarkCache::default();
            self.block_preview_cache.clear();
            self.commonmark_cache_bytes = 0;
            "Preview cache reset (over 300 MiB)".clone_into(&mut self.status);
        }
    }

    /// Render one non-blocking conflict window per pending conflict.
    ///
    /// Each window offers two actions:
    /// * **Reload from disk** — replaces the in-memory buffer with the
    ///   on-disk version, clears `dirty`, and drops the pending entry.
    /// * **Keep mine** — drops the pending entry without changing the
    ///   buffer; the next auto-save sweep overwrites the disk version.
    fn render_conflict_modals(&mut self, context: &egui::Context) {
        if self.pending_conflicts.is_empty() {
            // Once every modal has been resolved, allow the next watcher
            // burst to refill `pending_conflicts` cleanly by clearing the
            // overflow counter.
            self.pending_conflicts_dropped = 0;
            return;
        }
        let Some(folder) = self.folder.as_mut() else {
            // No folder, nothing to reconcile against. Drop conflicts so a
            // pending list does not survive a folder switch.
            self.pending_conflicts.clear();
            self.pending_conflicts_dropped = 0;
            return;
        };

        // Walk a snapshot so we can mutate `pending_conflicts` inside the loop.
        let pending = self.pending_conflicts.clone();
        let dropped = self.pending_conflicts_dropped;
        let mut resolved: Vec<NoteId> = Vec::new();
        let mut status_update: Option<String> = None;

        for note_id in pending {
            let Some(index) = folder.notes.iter().position(|note| note.note_id == note_id) else {
                // Note no longer in the folder (renamed, deleted). Drop the entry.
                resolved.push(note_id);
                continue;
            };
            let title = format!("{} changed on disk", note_id.file_stem());
            let window_id = egui::Id::new(("sm-conflict", note_id.relative_path()));
            let mut action: Option<ConflictChoice> = None;

            // Intentionally not passing `.open(&mut bool)`: without it egui
            // does not render a window-level close (X) button, so the user
            // must explicitly choose Reload or Keep mine. Closing via the OS
            // chrome would otherwise be ambiguous, and silently mapping it
            // to "Keep mine" was destructive — the next auto-save would
            // overwrite the on-disk version the user may have intended to
            // keep. The window is also non-collapsible / non-resizable /
            // non-movable to keep the affordance focused on the choice.
            egui::Window::new(title)
                .id(window_id)
                .collapsible(false)
                .resizable(false)
                .movable(false)
                .show(context, |ui| {
                    ui.label(
                        "This file was modified outside Sideromelane while \
                         your buffer has unsaved edits.",
                    );
                    if dropped > 0 {
                        ui.label(format!(
                            "+ {dropped} more pending conflicts (resolve open \
                             ones first)"
                        ));
                    }
                    ui.horizontal(|ui| {
                        if ui.button("Reload from disk").clicked() {
                            action = Some(ConflictChoice::Reload);
                        }
                        if ui.button("Keep mine").clicked() {
                            action = Some(ConflictChoice::Keep);
                        }
                    });
                });

            match action {
                Some(ConflictChoice::Reload) => {
                    let note = &mut folder.notes[index];
                    match fs::read_to_string(&note.absolute_path) {
                        Ok(source) => {
                            note.source = source;
                            note.dirty = false;
                            note.last_edit_at = Instant::now();
                            status_update = Some(format!(
                                "Reloaded {} from disk",
                                note.note_id.relative_path().display()
                            ));
                            resolved.push(note_id);
                        }
                        Err(error) => {
                            status_update = Some(format!("Reload failed: {error}"));
                            // Leave the entry pending so the user can retry.
                        }
                    }
                }
                Some(ConflictChoice::Keep) => {
                    resolved.push(note_id);
                }
                None => {}
            }
        }

        if !resolved.is_empty() {
            self.pending_conflicts.retain(|id| !resolved.contains(id));
        }
        if let Some(message) = status_update {
            self.status = message;
        }
    }

    /// Selects the note whose file stem matches `name` (case-sensitive). No-op if the
    /// folder is unloaded or the name is not found.
    fn navigate_to_note_by_name(&mut self, name: &str) {
        // Strip any trailing `#anchor` — anchor support is future work.
        let stem = name.split('#').next().unwrap_or(name);
        if let Some(folder) = self.folder.as_mut()
            && let Some(idx) = folder
                .notes
                .iter()
                .position(|note| note.note_id.file_stem() == stem)
        {
            folder.selected = Some(idx);
            self.active_block_index = None;
        }
    }
}

fn select_note(folder: &mut FolderState, note_id: &NoteId) {
    folder.selected = folder
        .notes
        .iter()
        .position(|note| &note.note_id == note_id);
}

const TREE_INDENT_PER_LEVEL: f32 = 14.0;

fn render_note_row(
    ui: &mut egui::Ui,
    folder: &FolderState,
    selected_note: &mut Option<usize>,
    note_id: &NoteId,
    depth: usize,
) {
    let Some(index) = folder.notes.iter().position(|n| &n.note_id == note_id) else {
        return;
    };
    let record = &folder.notes[index];
    let label = if record.dirty {
        format!("{} *", record.note_id.file_stem())
    } else {
        record.note_id.file_stem().to_owned()
    };
    ui.horizontal(|ui| {
        #[allow(clippy::cast_precision_loss)]
        ui.add_space(depth as f32 * TREE_INDENT_PER_LEVEL);
        if ui
            .selectable_label(*selected_note == Some(index), label)
            .clicked()
        {
            *selected_note = Some(index);
        }
    });
}

fn render_dir(
    ui: &mut egui::Ui,
    folder: &mut FolderState,
    selected_note: &mut Option<usize>,
    dir: &tree::DirNode,
    depth: usize,
    tree_changed: &mut bool,
) {
    let explicit_index = folder
        .settings
        .ui
        .tree_expanded_paths
        .iter()
        .position(|path| path == &dir.relative_path);
    let auto_expanded = folder.auto_expanded.contains(&dir.relative_path);
    let mut expanded = explicit_index.is_some() || auto_expanded;

    ui.horizontal(|ui| {
        #[allow(clippy::cast_precision_loss)]
        ui.add_space(depth as f32 * TREE_INDENT_PER_LEVEL);
        let chevron = if expanded { "\u{25BE}" } else { "\u{25B8}" }; // ▾ / ▸
        if ui
            .button(format!("{chevron} \u{1F4C1} {}", dir.name))
            .clicked()
        {
            expanded = !expanded;
            if expanded {
                if explicit_index.is_none() {
                    folder
                        .settings
                        .ui
                        .tree_expanded_paths
                        .push(dir.relative_path.clone());
                    *tree_changed = true;
                }
            } else {
                if let Some(index) = explicit_index {
                    folder.settings.ui.tree_expanded_paths.remove(index);
                    *tree_changed = true;
                }
                // Drop the auto-expand entry too so the next frame doesn't
                // immediately re-expand the directory the user just collapsed.
                folder.auto_expanded.remove(&dir.relative_path);
            }
        }
    });

    if !expanded {
        return;
    }

    for note_id in &dir.notes {
        render_note_row(ui, folder, selected_note, note_id, depth + 1);
    }
    for subdir in &dir.subdirs {
        render_dir(ui, folder, selected_note, subdir, depth + 1, tree_changed);
    }
}

/// Converts a split `ratio` and an available `total_height` into the pixel
/// height for the Files section, ensuring both Files and Search each have at
/// least 80 px and the handle itself is accounted for.
fn clamp_split_height(ratio: f32, total_height: f32) -> f32 {
    (ratio * total_height).clamp(80.0, total_height - 80.0 - HANDLE_PX)
}

fn walk_options_for(settings: &FolderSettings) -> WalkOptions {
    WalkOptions {
        include_dotfiles: settings.ignore.include_dotfiles,
        honor_gitignore: settings.ignore.honor_gitignore,
        ..WalkOptions::default()
    }
}

/// Pick a single note to surface as the initial editable view.
///
/// Prefers `Untitled.md` at the folder root if it exists; otherwise the
/// first Markdown file found by a shallow read of the root directory.
/// Returns `Ok(None)` for a folder with no Markdown files, and surfaces
/// IO errors only when the root itself cannot be read.
fn initial_note(root: &Path) -> std_io::Result<Option<NoteRecord>> {
    let preferred = root.join("Untitled.md");
    if preferred.is_file()
        && let Ok(record) = NoteRecord::read(root, preferred)
    {
        return Ok(Some(record));
    }

    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file()
            && path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
            && let Ok(record) = NoteRecord::read(root, path)
        {
            return Ok(Some(record));
        }
    }

    Ok(None)
}

fn read_image_header(path: &Path, buffer: &mut [u8; 16]) -> std_io::Result<usize> {
    let file = File::open(path)?;
    let mut reader = file.take(IMAGE_HEADER_PEEK);
    let mut total = 0;
    while total < buffer.len() {
        let read = reader.read(&mut buffer[total..])?;
        if read == 0 {
            break;
        }
        total += read;
    }
    Ok(total)
}

/// Reconcile the indexer's freshly discovered notes with the current
/// in-memory list. Existing dirty notes win — we do not overwrite an
/// in-memory edit with whatever the indexer just read from disk. Notes
/// the indexer found that the UI doesn't have are appended.
fn merge_discovered_notes(folder: &mut FolderState, discovered: Vec<indexer::NoteRecord>) {
    let selected_id = folder
        .selected
        .and_then(|index| folder.notes.get(index))
        .map(|note| note.note_id.clone());

    let mut existing: std::collections::HashMap<NoteId, NoteRecord> = folder
        .notes
        .drain(..)
        .map(|note| (note.note_id.clone(), note))
        .collect();

    let mut merged: Vec<NoteRecord> = Vec::with_capacity(discovered.len() + existing.len());
    for record in discovered {
        if let Some(mut existing_note) = existing.remove(&record.note_id) {
            existing_note.absolute_path = record.absolute_path;
            if !existing_note.dirty {
                existing_note.source = record.source;
            }
            merged.push(existing_note);
        } else {
            merged.push(NoteRecord {
                note_id: record.note_id,
                absolute_path: record.absolute_path,
                source: record.source,
                dirty: false,
                last_edit_at: Instant::now(),
            });
        }
    }
    for (_, leftover) in existing {
        // Carry over any unsaved-on-disk notes (e.g. fresh "Untitled" buffers).
        merged.push(leftover);
    }

    folder.selected =
        selected_id.and_then(|wanted| merged.iter().position(|note| note.note_id == wanted));
    if folder.selected.is_none() && !merged.is_empty() {
        folder.selected = Some(0);
    }
    folder.notes = merged;
    folder.rebuild_note_path_index();
}

/// Build a `(absolute_path -> index)` map for `notes`. Used as the watcher
/// event lookup table so `classify_watch_event` is O(1) per event.
fn build_note_path_index(notes: &[NoteRecord]) -> HashMap<PathBuf, usize> {
    notes
        .iter()
        .enumerate()
        .map(|(index, note)| (note.absolute_path.clone(), index))
        .collect()
}

/// Maximum number of `Untitled N.md` collisions we will probe before giving
/// up. A folder with this many existing untitled notes is so far past
/// reasonable that it almost certainly indicates a buggy caller or a
/// hostile filesystem rather than legitimate use.
const NEXT_UNTITLED_NOTE_MAX_ATTEMPTS: u32 = 10_000;

fn next_untitled_note(root: &Path) -> Option<(NoteId, PathBuf)> {
    for index in 0..NEXT_UNTITLED_NOTE_MAX_ATTEMPTS {
        let file_name = if index == 0 {
            String::from("Untitled.md")
        } else {
            format!("Untitled {index}.md")
        };
        let absolute_path = root.join(&file_name);

        if !absolute_path.exists()
            && let Ok(note_id) = NoteId::from_folder_relative_path(PathBuf::from(file_name))
        {
            return Some((note_id, absolute_path));
        }
    }

    None
}

fn is_image_path(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "gif" | "webp"
            )
        })
}

/// Canonicalize `target` for path-traversal checks.
///
/// Falls back to canonicalizing the parent directory and rejoining the leaf
/// name when the target itself does not yet exist (the common case for fresh
/// asset drops). Returns `None` if neither the target nor its parent can be
/// canonicalized.
fn canonicalize_target(target: &Path) -> Option<PathBuf> {
    if let Ok(path) = target.canonicalize() {
        return Some(path);
    }
    // Walk up until we find an ancestor that exists, canonicalize it, and
    // rejoin the missing tail. This handles fresh `assets/` directories that
    // `copy_asset` will create on demand.
    let mut tail: Vec<&std::ffi::OsStr> = Vec::new();
    let mut current = target;
    loop {
        if let Ok(canonical) = current.canonicalize() {
            let mut resolved = canonical;
            for component in tail.iter().rev() {
                resolved.push(component);
            }
            return Some(resolved);
        }
        let file_name = current.file_name()?;
        tail.push(file_name);
        current = current.parent()?;
    }
}

/// Maximum suffix attempts when resolving a unique asset path. A value this
/// large is well past anything a human would intentionally create and cheaply
/// caps a hostile or pathological assets/ directory.
const UNIQUE_ASSET_MAX_ATTEMPTS: u32 = 1024;

fn unique_asset_path(assets_dir: &Path, file_name: &str) -> Option<PathBuf> {
    let candidate = assets_dir.join(file_name);
    if !candidate.exists() {
        return Some(candidate);
    }

    let stem = Path::new(file_name)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("image");
    let extension = Path::new(file_name)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("png");

    for index in 1..=UNIQUE_ASSET_MAX_ATTEMPTS {
        let candidate = assets_dir.join(format!("{stem}-{index}.{extension}"));
        if !candidate.exists() {
            return Some(candidate);
        }
    }

    None
}

fn copy_asset(source_path: &Path, target_path: &Path) -> std_io::Result<()> {
    if let Some(parent) = target_path.parent() {
        fs::create_dir_all(parent)?;
    }

    fs::copy(source_path, target_path).map(|_bytes| ())
}

#[cfg(test)]
#[allow(clippy::float_cmp)]
mod tests {
    use super::clamp_split_height;

    #[test]
    fn clamp_split_height_midpoint() {
        assert_eq!(clamp_split_height(0.5, 200.0), 100.0);
    }

    #[test]
    fn clamp_split_height_high_ratio_clamped() {
        assert_eq!(clamp_split_height(0.99, 200.0), 116.0);
    }

    #[test]
    fn clamp_split_height_low_ratio_clamped() {
        assert_eq!(clamp_split_height(0.01, 200.0), 80.0);
    }
}
