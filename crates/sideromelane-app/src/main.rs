#![allow(missing_docs, clippy::too_many_lines)]

mod indexer;
mod io;
mod outline;

use std::fs::{self, File};
use std::io as std_io;
use std::io::Read;
use std::path::{Path, PathBuf};

use eframe::egui::{self, Color32, Pos2, Sense, Stroke, Vec2};
use sideromelane_core::{
    FolderIndex, FolderSettings, HybridSearchIndex, MarkdownNote, NoteAnalysis, NoteId,
    SearchQuery, WalkOptions, sanitize_asset_filename, validate_image_magic_bytes,
};

use crate::indexer::{Indexer, IndexerCommand, IndexerEvent};
use crate::io::safe_write;

/// Maximum byte size of an image that can be dropped into the assets folder.
const MAX_IMAGE_BYTES: u64 = 32 * 1024 * 1024;
/// Number of bytes inspected when validating image magic bytes.
const IMAGE_HEADER_PEEK: u64 = 16;
/// Maximum number of indexer events to drain per frame. Bounded so background bursts
/// cannot starve UI input handling.
const MAX_INDEXER_EVENTS_PER_FRAME: usize = 16;

fn main() -> eframe::Result {
    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Sideromelane")
            .with_inner_size([1200.0, 780.0]),
        ..Default::default()
    };

    eframe::run_native(
        "Sideromelane",
        native_options,
        Box::new(|_creation_context| Ok(Box::<SideromelaneApp>::default())),
    )
}

#[derive(Debug, Default)]
struct SideromelaneApp {
    folder: Option<FolderState>,
    mode: EditorMode,
    search_text: String,
    active_block_index: Option<usize>,
    status: String,
    indexer: Option<Indexer>,
}

impl eframe::App for SideromelaneApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if self.indexer.is_none() {
            self.indexer = Some(Indexer::new(ui.ctx().clone()));
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
                self.folder = Some(folder);
                self.active_block_index = None;
                self.dispatch_rescan(scan_root);
            }
            Err(error) => self.status = format!("Open failed: {error}"),
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

        let target_path = unique_asset_path(&assets_dir, &safe_name);

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

        let mut selected_note = folder.selected;
        egui::ScrollArea::vertical()
            .max_height(240.0)
            .show(ui, |ui| {
                for (index, note) in folder.notes.iter().enumerate() {
                    let label = if note.dirty {
                        format!("{} *", note.note_id.relative_path().display())
                    } else {
                        note.note_id.relative_path().display().to_string()
                    };
                    if ui
                        .selectable_label(selected_note == Some(index), label)
                        .clicked()
                    {
                        selected_note = Some(index);
                    }
                }
            });
        if selected_note != folder.selected {
            folder.selected = selected_note;
            self.active_block_index = None;
        }

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
        egui::ScrollArea::vertical().show(ui, |ui| {
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
                ui.horizontal(|ui| {
                    ui.add_space(outline::heading_indent_px(level));
                    let mut rich = egui::RichText::new(display)
                        .size(outline::heading_font_size(level, base_font));
                    if outline::heading_is_bold(level) {
                        rich = rich.strong();
                    }
                    ui.label(rich);
                });
            }
        }

        ui.separator();
        ui.heading("Graph");
        draw_graph(ui, folder);

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
            EditorMode::Raw => raw_editor(ui, &mut folder.notes[index], word_wrap),
            EditorMode::LivePreview => live_preview_editor(
                ui,
                &mut folder.notes[index],
                &mut self.active_block_index,
                word_wrap,
            ),
        };

        if changed {
            folder.notes[index].dirty = true;
            // Index refresh is deferred to save; typing must not block on
            // re-indexing every keystroke.
        }
    }
}

fn raw_editor(ui: &mut egui::Ui, note: &mut NoteRecord, word_wrap: bool) -> bool {
    let available_width = ui.available_width();
    if word_wrap {
        ui.add(
            egui::TextEdit::multiline(&mut note.source)
                .code_editor()
                .desired_width(available_width)
                .desired_rows(32)
                .lock_focus(true),
        )
        .changed()
    } else {
        // Horizontal scroll: don't constrain text wrap, let the scroll area handle overflow.
        let mut changed = false;
        egui::ScrollArea::horizontal().show(ui, |ui| {
            changed = ui
                .add(
                    egui::TextEdit::multiline(&mut note.source)
                        .code_editor()
                        .desired_width(f32::INFINITY)
                        .desired_rows(32)
                        .lock_focus(true),
                )
                .changed();
        });
        changed
    }
}

fn live_preview_editor(
    ui: &mut egui::Ui,
    note: &mut NoteRecord,
    active_block_index: &mut Option<usize>,
    word_wrap: bool,
) -> bool {
    let blocks = markdown_blocks(&note.source);
    let mut changed_block = None;
    let pane_width = ui.available_width();
    let active_block_width = if word_wrap { pane_width } else { f32::INFINITY };

    egui::ScrollArea::vertical().show(ui, |ui| {
        for (index, block) in blocks.iter().enumerate() {
            ui.group(|ui| {
                if *active_block_index == Some(index) {
                    let mut text = block.text.clone();
                    let response = ui.add(
                        egui::TextEdit::multiline(&mut text)
                            .code_editor()
                            .desired_width(active_block_width)
                            .desired_rows(block.text.lines().count().max(1)),
                    );
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

#[allow(clippy::cast_precision_loss)]
fn draw_graph(ui: &mut egui::Ui, folder: &mut FolderState) {
    let desired_size = Vec2::new(ui.available_width(), 220.0);
    let (rect, response) = ui.allocate_exact_size(desired_size, Sense::click());
    let painter = ui.painter_at(rect);
    let graph = folder.folder_index.graph();
    let nodes = graph.nodes();

    if nodes.is_empty() {
        return;
    }

    let center = rect.center();
    let radius = rect.width().min(rect.height()) * 0.38;
    let positions = nodes
        .iter()
        .enumerate()
        .map(|(index, node)| {
            let angle = index as f32 / nodes.len() as f32 * std::f32::consts::TAU;
            let position = Pos2::new(
                center.x + radius * angle.cos(),
                center.y + radius * angle.sin(),
            );
            (node.note_id().clone(), position)
        })
        .collect::<Vec<_>>();

    for edge in graph.edges() {
        let Some(source) = positions
            .iter()
            .find(|(note_id, _position)| note_id == edge.source())
            .map(|(_note_id, position)| *position)
        else {
            continue;
        };
        let Some(target) = positions
            .iter()
            .find(|(note_id, _position)| note_id == edge.target())
            .map(|(_note_id, position)| *position)
        else {
            continue;
        };
        painter.line_segment([source, target], Stroke::new(1.0, Color32::DARK_GRAY));
    }

    let selected_note = folder.selected_note().map(|note| note.note_id.clone());
    for (note_id, position) in &positions {
        let is_selected = selected_note.as_ref() == Some(note_id);
        painter.circle_filled(
            *position,
            if is_selected { 8.0 } else { 6.0 },
            if is_selected {
                Color32::from_rgb(180, 70, 70)
            } else {
                Color32::from_rgb(70, 110, 160)
            },
        );
        painter.text(
            *position + Vec2::new(8.0, -6.0),
            egui::Align2::LEFT_TOP,
            note_id.file_stem(),
            egui::FontId::proportional(11.0),
            Color32::WHITE,
        );
    }

    if response.clicked()
        && let Some(pointer) = response.interact_pointer_pos()
        && let Some((note_id, _position)) = positions
            .iter()
            .find(|(_note_id, position)| position.distance(pointer) <= 14.0)
    {
        select_note(folder, note_id);
    }
}

fn select_note(folder: &mut FolderState, note_id: &NoteId) {
    folder.selected = folder
        .notes
        .iter()
        .position(|note| &note.note_id == note_id);
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

fn unique_asset_path(assets_dir: &Path, file_name: &str) -> PathBuf {
    let candidate = assets_dir.join(file_name);
    if !candidate.exists() {
        return candidate;
    }

    let stem = Path::new(file_name)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("image");
    let extension = Path::new(file_name)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("png");

    for index in 1.. {
        let candidate = assets_dir.join(format!("{stem}-{index}.{extension}"));
        if !candidate.exists() {
            return candidate;
        }
    }

    unreachable!("unbounded loop returns before exhausting usize");
}

fn copy_asset(source_path: &Path, target_path: &Path) -> std_io::Result<()> {
    if let Some(parent) = target_path.parent() {
        fs::create_dir_all(parent)?;
    }

    fs::copy(source_path, target_path).map(|_bytes| ())
}
