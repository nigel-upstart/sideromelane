#![allow(missing_docs, clippy::too_many_lines)]

mod graph_layout;
mod graph_view;
mod indexer;
mod io;
mod outline;
mod preferences;
mod state;
mod tree;

use std::fs::{self, File};
use std::io as std_io;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use eframe::egui::{self, Sense};
use sideromelane_core::{
    FolderIndex, FolderSettings, HybridSearchIndex, MarkdownNote, NoteAnalysis, NoteId,
    SearchQuery, WalkOptions, sanitize_asset_filename, validate_image_magic_bytes,
};

use crate::indexer::{Indexer, IndexerCommand, IndexerEvent};
use crate::io::safe_write;
use crate::preferences::PreferencesWindow;
use crate::state::{AppState, StartupMode};

/// Maximum byte size of an image that can be dropped into the assets folder.
const MAX_IMAGE_BYTES: u64 = 32 * 1024 * 1024;
/// Number of bytes inspected when validating image magic bytes.
const IMAGE_HEADER_PEEK: u64 = 16;
/// Maximum number of indexer events to drain per frame. Bounded so background bursts
/// cannot starve UI input handling.
const MAX_INDEXER_EVENTS_PER_FRAME: usize = 16;
/// Debounce window for persisting [`AppState`]. Coalesces rapid edits (slider
/// drags, repeated folder opens) into a single atomic save without blocking
/// the UI on every keystroke.
const APP_STATE_SAVE_DEBOUNCE: Duration = Duration::from_millis(250);

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
            app_state,
            app_state_dirty: false,
            last_state_save: Instant::now(),
            preferences_open: false,
            preferences_window: PreferencesWindow::default(),
            startup_pending: true,
        }
    }
}

impl eframe::App for SideromelaneApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.indexer.is_none() {
            self.indexer = Some(Indexer::new(ui.ctx().clone()));
        }
        if self.startup_pending {
            self.startup_pending = false;
            self.run_startup();
        }
        self.drain_indexer_events();
        self.handle_dropped_files(ui.ctx());

        egui::Panel::top("top_bar").show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Open Folder").clicked() {
                    self.pick_folder();
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
        })
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
        })
    }
}

impl SideromelaneApp {
    fn pick_folder(&mut self) {
        let Some(root) = rfd::FileDialog::new().pick_folder() else {
            return;
        };

        self.open_folder(root);
    }

    fn open_folder(&mut self, root: PathBuf) {
        match FolderState::load(root) {
            Ok(folder) => {
                self.status = format!("Opened {}", folder.root.display());
                let scan_root = folder.root.clone();
                self.app_state.record_folder_open(&scan_root);
                // Drop the previous note's `last_note` so the post-`update`
                // bookkeeping picks up whatever this folder selects on the
                // next frame rather than carrying over a stale relative path.
                self.app_state.last_note = None;
                self.app_state_dirty = true;
                self.folder = Some(folder);
                self.active_block_index = None;
                self.dispatch_rescan(scan_root);
            }
            Err(error) => self.status = format!("Open failed: {error}"),
        }
    }

    /// Boot-time hand-off driven by `app_state.startup_mode`. Called once
    /// from `update` on the first frame so the eframe context is live in
    /// case any startup branch wants to surface a dialog.
    fn run_startup(&mut self) {
        match self.app_state.startup_mode {
            StartupMode::ReloadLast => {
                if let Some(folder) = self.app_state.last_folder.clone()
                    && folder.is_dir()
                {
                    let target_note = self.app_state.last_note.clone();
                    self.open_folder(folder);
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
                self.boot_default_folder();
            }
            StartupMode::NewNote => {
                self.boot_default_folder();
                self.new_note();
            }
        }
    }

    /// Open (creating if needed) the configured default folder.
    fn boot_default_folder(&mut self) {
        let default_folder = self.app_state.default_folder.clone();
        if let Err(error) = fs::create_dir_all(&default_folder) {
            self.status = format!(
                "Default folder unavailable ({}): {error}",
                default_folder.display()
            );
            return;
        }
        self.open_folder(default_folder);
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
        let (note_id, absolute_path) = next_untitled_note(&folder.root);
        let source = format!("# {}\n", note_id.file_stem());
        folder.notes.push(NoteRecord {
            note_id,
            absolute_path,
            source,
            dirty: true,
        });
        folder.selected = Some(folder.notes.len() - 1);
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
                let relative = note.note_id.relative_path().display().to_string();
                self.status = format!("Saved {relative}");
                if let Some(indexer) = self.indexer.as_ref() {
                    indexer.send(IndexerCommand::NoteChanged { note_id, source });
                }
            }
            Err(error) => self.status = format!("Save failed: {error}"),
        }
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

        egui::ScrollArea::vertical()
            .max_height(240.0)
            .id_salt("files_tree")
            .show(ui, |ui| {
                for note_id in &folder_tree.root_notes {
                    render_note_row(ui, folder, &mut selected_note, note_id, 0);
                }
                for subdir in &folder_tree.subdirs {
                    render_dir(ui, folder, &mut selected_note, subdir, 0, &mut tree_changed);
                }
            });
        folder.cached_tree = Some(folder_tree);

        if selected_note != folder.selected {
            folder.selected = selected_note;
            self.active_block_index = None;
        }

        let pending_settings_save = if tree_changed {
            Some(folder.root.clone())
        } else {
            None
        };

        ui.separator();
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

        ui.separator();
        ui.heading("Graph");
        let selected_note = folder.selected_note().map(|note| note.note_id.clone());
        let clicked = graph_view::draw(
            ui,
            &mut self.graph_view,
            &folder.folder_index,
            selected_note.as_ref(),
        );
        if let Some(note_id) = clicked {
            select_note(folder, &note_id);
            self.active_block_index = None;
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
            ),
        };

        if changed {
            folder.notes[index].dirty = true;
            // Index refresh is deferred to save; typing must not block on
            // re-indexing every keystroke.
        }
    }
}

/// Returns a layouter closure that prevents text wrapping. `LayoutJob::simple`
/// already sets `wrap.max_width` to the value we pass in, so `f32::INFINITY`
/// keeps lines on a single row.
fn nowrap_layouter()
-> impl FnMut(&egui::Ui, &dyn egui::TextBuffer, f32) -> std::sync::Arc<egui::Galley> {
    move |ui: &egui::Ui, text: &dyn egui::TextBuffer, _wrap_width: f32| {
        let font_id = egui::TextStyle::Monospace.resolve(ui.style());
        let job = egui::text::LayoutJob::simple(
            text.as_str().to_owned(),
            font_id,
            ui.visuals().text_color(),
            f32::INFINITY,
        );
        ui.painter().layout_job(job)
    }
}

fn raw_editor(
    ui: &mut egui::Ui,
    note: &mut NoteRecord,
    word_wrap: bool,
    pending_jump: &mut Option<usize>,
) -> bool {
    let available_width = ui.available_width();
    let apply_jump = |response: &egui::Response, offset: usize, ui: &egui::Ui| {
        use egui::text::{CCursor, CCursorRange};
        use egui::widgets::text_edit::TextEditState;
        let id = response.id;
        let mut state = TextEditState::load(ui.ctx(), id).unwrap_or_default();
        state
            .cursor
            .set_char_range(Some(CCursorRange::one(CCursor::new(offset))));
        state.store(ui.ctx(), id);
        response.request_focus();
        response.scroll_to_me(Some(egui::Align::Center));
    };
    if word_wrap {
        let response = ui.add(
            egui::TextEdit::multiline(&mut note.source)
                .code_editor()
                .desired_width(available_width)
                .desired_rows(32)
                .lock_focus(true),
        );
        if let Some(offset) = pending_jump.take() {
            apply_jump(&response, offset, ui);
        }
        response.changed()
    } else {
        // Horizontal scroll: don't constrain text wrap, let the scroll area handle overflow.
        let mut changed = false;
        let mut layouter = nowrap_layouter();
        let mut jump_response: Option<egui::Response> = None;
        egui::ScrollArea::horizontal().show(ui, |ui| {
            let response = ui.add(
                egui::TextEdit::multiline(&mut note.source)
                    .code_editor()
                    .desired_width(f32::INFINITY)
                    .desired_rows(32)
                    .lock_focus(true)
                    .layouter(&mut layouter),
            );
            changed = response.changed();
            jump_response = Some(response);
        });
        if let (Some(offset), Some(response)) = (pending_jump.take(), jump_response) {
            apply_jump(&response, offset, ui);
        }
        changed
    }
}

fn live_preview_editor(
    ui: &mut egui::Ui,
    note: &mut NoteRecord,
    active_block_index: &mut Option<usize>,
    word_wrap: bool,
    pending_jump: &mut Option<usize>,
) -> bool {
    let blocks = markdown_blocks(&note.source);

    // Resolve a pending jump to a block index before rendering.
    if let Some(offset) = pending_jump.take() {
        *active_block_index = blocks
            .iter()
            .position(|b| b.range.start <= offset && offset < b.range.end)
            .or(if blocks.is_empty() { None } else { Some(0) });
    }

    let mut changed_block = None;
    let pane_width = ui.available_width();
    let active_block_width = if word_wrap { pane_width } else { f32::INFINITY };

    egui::ScrollArea::vertical().show(ui, |ui| {
        for (index, block) in blocks.iter().enumerate() {
            let group_response = ui.group(|ui| {
                if *active_block_index == Some(index) {
                    let mut text = block.text.clone();
                    let mut layouter = nowrap_layouter();
                    let widget = egui::TextEdit::multiline(&mut text)
                        .code_editor()
                        .desired_width(active_block_width)
                        .desired_rows(block.text.lines().count().max(1));
                    let response = if word_wrap {
                        ui.add(widget)
                    } else {
                        ui.add(widget.layouter(&mut layouter))
                    };
                    if response.changed() {
                        changed_block = Some((block.range.clone(), text));
                    }
                } else {
                    let response = render_block(ui, block);
                    if response.clicked() {
                        *active_block_index = Some(index);
                    }
                }
            });
            // Scroll the active block into view when it was just activated via
            // a pending jump.
            if *active_block_index == Some(index) {
                group_response
                    .response
                    .scroll_to_me(Some(egui::Align::Center));
            }
        }
    });

    if let Some((range, text)) = changed_block {
        note.source.replace_range(range, &text);
        true
    } else {
        false
    }
}

fn render_block(ui: &mut egui::Ui, block: &MarkdownBlock) -> egui::Response {
    match block.kind {
        MarkdownBlockKind::Frontmatter => {
            ui.vertical(|ui| {
                for line in block.text.lines().filter(|line| !line.trim().is_empty()) {
                    if line.trim() != "---" {
                        ui.monospace(line);
                    }
                }
            })
            .response
        }
        MarkdownBlockKind::Heading(level) => {
            let text = block.text.trim_start_matches('#').trim();
            if level == 1 {
                ui.heading(text)
            } else {
                ui.strong(text)
            }
        }
        MarkdownBlockKind::Code | MarkdownBlockKind::Table => ui.add(
            egui::Label::new(egui::RichText::new(&block.text).monospace()).sense(Sense::click()),
        ),
        MarkdownBlockKind::List | MarkdownBlockKind::Paragraph | MarkdownBlockKind::Blank => {
            ui.add(egui::Label::new(preview_text(&block.text)).sense(Sense::click()))
        }
    }
}

#[derive(Debug, Clone)]
struct MarkdownBlock {
    range: std::ops::Range<usize>,
    text: String,
    kind: MarkdownBlockKind,
}

#[derive(Debug, Clone, Copy)]
enum MarkdownBlockKind {
    Blank,
    Code,
    Frontmatter,
    Heading(u8),
    List,
    Paragraph,
    Table,
}

fn markdown_blocks(source: &str) -> Vec<MarkdownBlock> {
    let line_ranges = line_ranges(source);
    let mut blocks = Vec::new();
    let mut index = if source.starts_with("---")
        && let Some(frontmatter_end) = line_ranges
            .iter()
            .skip(1)
            .position(|range| source[range.clone()].trim_end_matches(['\r', '\n']).trim() == "---")
    {
        let end_line = frontmatter_end + 1;
        let range = 0..line_ranges[end_line].end;
        blocks.push(block(source, range, MarkdownBlockKind::Frontmatter));
        end_line + 1
    } else {
        0
    };

    while index < line_ranges.len() {
        let range = line_ranges[index].clone();
        let line = source[range.clone()].trim_end_matches(['\r', '\n']);
        let trimmed = line.trim();

        if trimmed.is_empty() {
            blocks.push(block(source, range, MarkdownBlockKind::Blank));
            index += 1;
        } else if trimmed.starts_with("```") {
            let start = index;
            index += 1;
            while index < line_ranges.len()
                && !source[line_ranges[index].clone()]
                    .trim_start()
                    .starts_with("```")
            {
                index += 1;
            }
            if index < line_ranges.len() {
                index += 1;
            }
            blocks.push(block(
                source,
                line_ranges[start].start..line_ranges[index - 1].end,
                MarkdownBlockKind::Code,
            ));
        } else if let Some(level) = heading_level(trimmed) {
            blocks.push(block(source, range, MarkdownBlockKind::Heading(level)));
            index += 1;
        } else if trimmed.starts_with('|') {
            let start = index;
            index += 1;
            while index < line_ranges.len()
                && source[line_ranges[index].clone()]
                    .trim_start()
                    .starts_with('|')
            {
                index += 1;
            }
            blocks.push(block(
                source,
                line_ranges[start].start..line_ranges[index - 1].end,
                MarkdownBlockKind::Table,
            ));
        } else if is_list_line(trimmed) {
            let start = index;
            index += 1;
            while index < line_ranges.len()
                && is_list_line(source[line_ranges[index].clone()].trim())
            {
                index += 1;
            }
            blocks.push(block(
                source,
                line_ranges[start].start..line_ranges[index - 1].end,
                MarkdownBlockKind::List,
            ));
        } else {
            let start = index;
            index += 1;
            while index < line_ranges.len() {
                let next_line = source[line_ranges[index].clone()].trim();
                if next_line.is_empty()
                    || heading_level(next_line).is_some()
                    || next_line.starts_with("```")
                    || next_line.starts_with('|')
                    || is_list_line(next_line)
                {
                    break;
                }
                index += 1;
            }
            blocks.push(block(
                source,
                line_ranges[start].start..line_ranges[index - 1].end,
                MarkdownBlockKind::Paragraph,
            ));
        }
    }

    if blocks.is_empty() {
        blocks.push(block(source, 0..source.len(), MarkdownBlockKind::Blank));
    }

    blocks
}

fn block(source: &str, range: std::ops::Range<usize>, kind: MarkdownBlockKind) -> MarkdownBlock {
    MarkdownBlock {
        text: source[range.clone()].to_owned(),
        range,
        kind,
    }
}

fn line_ranges(source: &str) -> Vec<std::ops::Range<usize>> {
    let mut ranges = Vec::new();
    let mut start = 0;

    for line in source.split_inclusive('\n') {
        let end = start + line.len();
        ranges.push(start..end);
        start = end;
    }

    if start < source.len() || source.is_empty() {
        ranges.push(start..source.len());
    }

    ranges
}

fn heading_level(line: &str) -> Option<u8> {
    let level = line.bytes().take_while(|byte| *byte == b'#').count();

    ((1..=6).contains(&level) && line.as_bytes().get(level) == Some(&b' '))
        .then(|| u8::try_from(level).ok())
        .flatten()
}

fn is_list_line(line: &str) -> bool {
    line.starts_with("- ") || line.starts_with("* ") || line.starts_with("+ ")
}

fn preview_text(source: &str) -> String {
    source
        .replace("- [ ]", "[ ]")
        .replace("- [x]", "[x]")
        .replace("- [X]", "[x]")
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
}

fn next_untitled_note(root: &Path) -> (NoteId, PathBuf) {
    for index in 0.. {
        let file_name = if index == 0 {
            String::from("Untitled.md")
        } else {
            format!("Untitled {index}.md")
        };
        let absolute_path = root.join(&file_name);

        if !absolute_path.exists()
            && let Ok(note_id) = NoteId::from_folder_relative_path(PathBuf::from(file_name))
        {
            return (note_id, absolute_path);
        }
    }

    unreachable!("unbounded loop returns before exhausting usize");
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
