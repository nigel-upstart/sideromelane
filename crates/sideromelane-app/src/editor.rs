//! Editor surface: raw and live-preview text editing helpers extracted from
//! `main.rs`. The Markdown block segmentation, the wiki-link registration
//! pass, and the cursor-jump apply helpers all live here so `main.rs` can
//! stay focused on app-level orchestration.

use std::path::Path;

use eframe::egui::{self, Sense};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};

use crate::NoteRecord;
use crate::preview::{NOTE_LINK_SCHEME, transform_wiki_links};

/// Returns a layouter closure that prevents text wrapping. `LayoutJob::simple`
/// already sets `wrap.max_width` to the value we pass in, so `f32::INFINITY`
/// keeps lines on a single row.
pub fn nowrap_layouter()
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

pub fn raw_editor(
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

#[allow(clippy::too_many_arguments)]
pub fn live_preview_editor(
    ui: &mut egui::Ui,
    note: &mut NoteRecord,
    active_block_index: &mut Option<usize>,
    word_wrap: bool,
    pending_jump: &mut Option<usize>,
    folder_root: &Path,
    cache: &mut CommonMarkCache,
    pending_link_click: &mut Option<String>,
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

    let note_stem = note.note_id.file_stem().to_owned();

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
                    // Pre-pass: rewrite wiki links and image embeds to CommonMark.
                    let transformed = transform_wiki_links(&block.text, folder_root);

                    // Register every in-app link so `egui_commonmark` routes clicks
                    // through the cache instead of the OS browser.
                    register_note_links(cache, &transformed);

                    // The viewer needs a stable, unique source id per block to keep
                    // its scrollable state. Combine the note stem with the block index.
                    let source_id =
                        egui::Id::new(("sm-mdblock", &note_stem, index, block.range.start));
                    let response = ui
                        .push_id(source_id, |ui| {
                            CommonMarkViewer::new().show(ui, cache, &transformed);
                        })
                        .response
                        .interact(Sense::click());

                    // Drain any link hooks that activated this frame. The first match
                    // wins; subsequent clicks queue behind via `pending_link_click`.
                    if pending_link_click.is_none()
                        && let Some(url) = take_clicked_note_link(cache)
                    {
                        *pending_link_click = Some(url);
                    }

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

/// Registers every `sideromelane://note/...` URL appearing in `text` with the cache so
/// `egui_commonmark` renders them as in-app links rather than OS-browser hyperlinks.
pub fn register_note_links(cache: &mut CommonMarkCache, text: &str) {
    let mut cursor = 0;
    while let Some(rel) = text[cursor..].find(NOTE_LINK_SCHEME) {
        let start = cursor + rel;
        let after = start + NOTE_LINK_SCHEME.len();
        // The URL ends at the first character disallowed in a Markdown link target.
        let end = text[after..]
            .find([')', ' ', '\n', '\r', '\t', '"', '<', '>'])
            .map_or(text.len(), |rel_end| after + rel_end);
        let url = &text[start..end];
        if cache.get_link_hook(url).is_none() {
            cache.add_link_hook(url);
        }
        cursor = end;
    }
}

/// Returns the first registered note link that was clicked this frame, stripped of its
/// scheme. Resets the hook so subsequent frames don't re-fire it.
pub fn take_clicked_note_link(cache: &mut CommonMarkCache) -> Option<String> {
    let clicked: Option<String> = cache
        .link_hooks()
        .iter()
        .find_map(|(url, hit)| hit.then(|| url.clone()));
    clicked.and_then(|url| {
        // Reset the hook flag so we don't re-trigger next frame.
        cache.link_hooks_mut().insert(url.clone(), false);
        url.strip_prefix(NOTE_LINK_SCHEME).map(str::to_owned)
    })
}

#[derive(Debug, Clone)]
pub struct MarkdownBlock {
    pub range: std::ops::Range<usize>,
    pub text: String,
}

pub fn markdown_blocks(source: &str) -> Vec<MarkdownBlock> {
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
        blocks.push(block(source, range));
        end_line + 1
    } else {
        0
    };

    while index < line_ranges.len() {
        let range = line_ranges[index].clone();
        let line = source[range.clone()].trim_end_matches(['\r', '\n']);
        let trimmed = line.trim();

        if trimmed.is_empty() {
            blocks.push(block(source, range));
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
            ));
        } else if heading_level(trimmed).is_some() {
            blocks.push(block(source, range));
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
            ));
        }
    }

    if blocks.is_empty() {
        blocks.push(block(source, 0..source.len()));
    }

    blocks
}

fn block(source: &str, range: std::ops::Range<usize>) -> MarkdownBlock {
    MarkdownBlock {
        text: source[range.clone()].to_owned(),
        range,
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
