#![allow(missing_docs, clippy::too_many_lines)]

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use eframe::egui::{self, Color32, Pos2, Sense, Stroke, Vec2};
use sideromelane_core::{
    HybridSearchIndex, MarkdownNote, NoteAnalysis, NoteId, SearchQuery, VaultIndex,
};

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
    vault: Option<VaultState>,
    mode: EditorMode,
    search_text: String,
    active_block_index: Option<usize>,
    status: String,
}

impl eframe::App for SideromelaneApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.handle_dropped_files(ui.ctx());

        egui::Panel::top("top_bar").show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                if ui.button("Open Vault").clicked() {
                    self.pick_vault();
                }
                let can_use_vault = self.vault.is_some();
                if ui
                    .add_enabled(can_use_vault, egui::Button::new("New"))
                    .clicked()
                {
                    self.new_note();
                }
                if ui
                    .add_enabled(can_use_vault, egui::Button::new("Save"))
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
struct VaultState {
    root: PathBuf,
    notes: Vec<NoteRecord>,
    selected: Option<usize>,
    search_index: HybridSearchIndex,
    vault_index: VaultIndex,
}

impl VaultState {
    fn load(root: PathBuf) -> io::Result<Self> {
        let mut paths = Vec::new();
        collect_markdown_paths(&root, &mut paths)?;
        paths.sort();

        let notes = paths
            .into_iter()
            .filter_map(|path| NoteRecord::read(&root, path).ok())
            .collect::<Vec<_>>();
        let mut vault = Self {
            root,
            notes,
            selected: None,
            search_index: HybridSearchIndex::default(),
            vault_index: VaultIndex::default(),
        };
        vault.selected = (!vault.notes.is_empty()).then_some(0);
        vault.rebuild_indexes();

        Ok(vault)
    }

    fn parsed_notes(&self) -> Vec<MarkdownNote> {
        self.notes
            .iter()
            .map(|note| MarkdownNote::parse(note.note_id.clone(), note.source.clone()))
            .collect()
    }

    fn rebuild_indexes(&mut self) {
        let notes = self.parsed_notes();
        self.search_index = HybridSearchIndex::from_notes(notes.clone());
        self.vault_index = VaultIndex::from_notes(notes);
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
    fn read(root: &Path, absolute_path: PathBuf) -> io::Result<Self> {
        let relative_path = absolute_path
            .strip_prefix(root)
            .map_err(io::Error::other)?
            .to_path_buf();
        let note_id = NoteId::from_vault_relative_path(relative_path).map_err(io::Error::other)?;
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
    fn pick_vault(&mut self) {
        let Some(root) = rfd::FileDialog::new().pick_folder() else {
            return;
        };

        self.open_vault(root);
    }

    fn open_vault(&mut self, root: PathBuf) {
        match VaultState::load(root) {
            Ok(vault) => {
                self.status = format!("Opened {}", vault.root.display());
                self.vault = Some(vault);
                self.active_block_index = None;
            }
            Err(error) => self.status = format!("Open failed: {error}"),
        }
    }

    fn new_note(&mut self) {
        let Some(vault) = self.vault.as_mut() else {
            return;
        };
        let (note_id, absolute_path) = next_untitled_note(&vault.root);
        let source = format!("# {}\n", note_id.file_stem());
        vault.notes.push(NoteRecord {
            note_id,
            absolute_path,
            source,
            dirty: true,
        });
        vault.selected = Some(vault.notes.len() - 1);
        vault.rebuild_indexes();
        self.active_block_index = None;
    }

    fn save_selected(&mut self) {
        let Some(vault) = self.vault.as_mut() else {
            return;
        };
        let Some(note) = vault.selected_note_mut() else {
            return;
        };

        match safe_write(&note.absolute_path, &note.source) {
            Ok(()) => {
                note.dirty = false;
                self.status = format!("Saved {}", note.note_id.relative_path().display());
                vault.rebuild_indexes();
            }
            Err(error) => self.status = format!("Save failed: {error}"),
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
        let Some(vault) = self.vault.as_mut() else {
            return;
        };
        let assets_dir = vault.root.join("assets");
        let Some(file_name) = source_path.file_name().and_then(|name| name.to_str()) else {
            return;
        };
        let target_path = unique_asset_path(&assets_dir, file_name);

        match copy_asset(source_path, &target_path) {
            Ok(()) => {
                let relative_target = target_path
                    .strip_prefix(&vault.root)
                    .unwrap_or(&target_path)
                    .to_string_lossy()
                    .into_owned();
                let Some(note) = vault.selected_note_mut() else {
                    return;
                };
                note.source.push('\n');
                note.source.push_str("![[");
                note.source.push_str(&relative_target);
                note.source.push_str("]]\n");
                note.dirty = true;
                self.status = format!("Inserted {relative_target}");
                vault.rebuild_indexes();
            }
            Err(error) => self.status = format!("Image copy failed: {error}"),
        }
    }

    fn left_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Files");
        let Some(vault) = self.vault.as_mut() else {
            return;
        };

        let mut selected_note = vault.selected;
        egui::ScrollArea::vertical()
            .max_height(240.0)
            .show(ui, |ui| {
                for (index, note) in vault.notes.iter().enumerate() {
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
        if selected_note != vault.selected {
            vault.selected = selected_note;
            self.active_block_index = None;
        }

        ui.separator();
        ui.heading("Search");
        let search_changed = ui
            .add(egui::TextEdit::singleline(&mut self.search_text).hint_text("Search"))
            .changed();
        if search_changed {
            vault.rebuild_indexes();
        }

        let query = if self.search_text.trim().is_empty() {
            SearchQuery::empty()
        } else {
            SearchQuery::text(self.search_text.clone())
        };
        let results = vault.search_index.search(&query);
        egui::ScrollArea::vertical().show(ui, |ui| {
            for result in results {
                if let Some(index) = vault
                    .notes
                    .iter()
                    .position(|note| &note.note_id == result.note_id())
                {
                    let note = &vault.notes[index];
                    if ui
                        .selectable_label(
                            vault.selected == Some(index),
                            format!(
                                "{} ({:.1})",
                                note.note_id.file_stem(),
                                result.combined_score()
                            ),
                        )
                        .clicked()
                    {
                        vault.selected = Some(index);
                        self.active_block_index = None;
                    }
                }
            }
        });
    }

    fn right_panel(&mut self, ui: &mut egui::Ui) {
        let Some(vault) = self.vault.as_mut() else {
            return;
        };

        ui.heading("Backlinks");
        if let Some(note) = vault.selected_note() {
            let backlink_sources = vault
                .vault_index
                .backlinks_to(&note.note_id)
                .iter()
                .map(|backlink| backlink.source().clone())
                .collect::<Vec<_>>();
            for source in backlink_sources {
                if ui.button(source.file_stem()).clicked() {
                    select_note(vault, &source);
                    self.active_block_index = None;
                }
            }
        }

        ui.separator();
        ui.heading("Outline");
        if let Some(parsed_note) = vault.selected_parsed_note() {
            let analysis = NoteAnalysis::from_note(&parsed_note);
            for heading in analysis.headings() {
                ui.label(format!(
                    "{} {}",
                    "#".repeat(usize::from(heading.level())),
                    heading.text()
                ));
            }
        }

        ui.separator();
        ui.heading("Graph");
        draw_graph(ui, vault);
    }

    fn main_panel(&mut self, ui: &mut egui::Ui) {
        let Some(vault) = self.vault.as_mut() else {
            ui.centered_and_justified(|ui| {
                ui.heading("Sideromelane");
            });
            return;
        };
        let Some(index) = vault.selected else {
            ui.centered_and_justified(|ui| {
                ui.heading("No Notes");
            });
            return;
        };

        ui.horizontal(|ui| {
            let note = &vault.notes[index];
            ui.heading(note.note_id.file_stem());
            if note.dirty {
                ui.label("Unsaved");
            }
        });
        ui.separator();

        let changed = match self.mode {
            EditorMode::Raw => raw_editor(ui, &mut vault.notes[index]),
            EditorMode::LivePreview => {
                live_preview_editor(ui, &mut vault.notes[index], &mut self.active_block_index)
            }
        };

        if changed {
            vault.notes[index].dirty = true;
            vault.rebuild_indexes();
        }
    }
}

fn raw_editor(ui: &mut egui::Ui, note: &mut NoteRecord) -> bool {
    ui.add(
        egui::TextEdit::multiline(&mut note.source)
            .code_editor()
            .desired_rows(32)
            .lock_focus(true),
    )
    .changed()
}

fn live_preview_editor(
    ui: &mut egui::Ui,
    note: &mut NoteRecord,
    active_block_index: &mut Option<usize>,
) -> bool {
    let blocks = markdown_blocks(&note.source);
    let mut changed_block = None;

    egui::ScrollArea::vertical().show(ui, |ui| {
        for (index, block) in blocks.iter().enumerate() {
            ui.group(|ui| {
                if *active_block_index == Some(index) {
                    let mut text = block.text.clone();
                    let response = ui.add(
                        egui::TextEdit::multiline(&mut text)
                            .code_editor()
                            .desired_width(f32::INFINITY)
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
fn draw_graph(ui: &mut egui::Ui, vault: &mut VaultState) {
    let desired_size = Vec2::new(ui.available_width(), 220.0);
    let (rect, response) = ui.allocate_exact_size(desired_size, Sense::click());
    let painter = ui.painter_at(rect);
    let graph = vault.vault_index.graph();
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

    let selected_note = vault.selected_note().map(|note| note.note_id.clone());
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
        select_note(vault, note_id);
    }
}

fn select_note(vault: &mut VaultState, note_id: &NoteId) {
    vault.selected = vault.notes.iter().position(|note| &note.note_id == note_id);
}

fn collect_markdown_paths(root: &Path, paths: &mut Vec<PathBuf>) -> io::Result<()> {
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            collect_markdown_paths(&path, paths)?;
        } else if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
        {
            paths.push(path);
        }
    }

    Ok(())
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
            && let Ok(note_id) = NoteId::from_vault_relative_path(PathBuf::from(file_name))
        {
            return (note_id, absolute_path);
        }
    }

    unreachable!("unbounded loop returns before exhausting usize");
}

fn safe_write(path: &Path, source: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary_path = path.with_extension("md.tmp");

    fs::write(&temporary_path, source)?;
    fs::rename(temporary_path, path)
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

fn copy_asset(source_path: &Path, target_path: &Path) -> io::Result<()> {
    if let Some(parent) = target_path.parent() {
        fs::create_dir_all(parent)?;
    }

    fs::copy(source_path, target_path).map(|_bytes| ())
}
