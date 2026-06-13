//! Editor surface: raw and live-preview text editing helpers extracted from
//! `main.rs`. The Markdown block segmentation, the wiki-link registration
//! pass, and the cursor-jump apply helpers all live here so `main.rs` can
//! stay focused on app-level orchestration.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::ops::Range;
use std::path::Path;

use eframe::egui::{self, Sense};
use egui_commonmark::{CommonMarkCache, CommonMarkViewer};
use sideromelane_core::FolderIndex;

use crate::NoteRecord;
use crate::preview::{NOTE_LINK_SCHEME, transform_wiki_links};
use crate::typography::{PreviewReadingFont, apply_preview_text_style};
use crate::wiki_link_popup::{WikiLinkAction, WikiLinkPopup};

/// Cached output of the per-block live-preview pre-pass. `transformed` is the
/// `CommonMark`-shaped text that gets fed to `egui_commonmark`; `link_targets`
/// is the precomputed set of `sideromelane://note/...` URLs registered with
/// the cache so the per-frame pass does not have to re-scan the rendered
/// text every frame.
#[derive(Debug, Clone)]
pub struct CachedBlockPreview {
    pub transformed: String,
    pub link_targets: Vec<String>,
}

/// Per-block memo keyed by `(byte range in current source, fast hash of text)`.
/// Stale entries (mismatching range, or text mutated under a stable range)
/// fall through automatically because the key changes; the live-preview
/// renderer also sweeps stale `range` keys at the start of each frame.
pub type BlockPreviewCache = HashMap<(Range<usize>, u64), CachedBlockPreview>;

/// Hash a block's text into a 64-bit fingerprint used as part of the cache
/// key. `DefaultHasher` is allocation-free and fast enough for per-block
/// hashing of small markdown chunks; collisions only mean a recompute, never
/// a correctness bug.
#[must_use]
pub fn hash_block_text(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

/// Look up the cached pre-pass output for a block, computing it via
/// `compute` if absent. The closure is invoked at most once per
/// `(range, hash)` key so live preview can amortize `transform_wiki_links`
/// + link extraction across frames.
pub fn get_or_insert_cached_preview<F>(
    cache: &mut BlockPreviewCache,
    range: Range<usize>,
    hash: u64,
    compute: F,
) -> &CachedBlockPreview
where
    F: FnOnce() -> CachedBlockPreview,
{
    cache.entry((range, hash)).or_insert_with(compute)
}

/// Drop entries whose range is not present in `current_ranges`. Called at the
/// start of every live-preview frame so the cache stays bounded by the
/// current block layout. This is O(`blocks` * `cache_entries`) but both sides
/// are small per note in practice.
pub fn sweep_stale_block_previews(cache: &mut BlockPreviewCache, current_ranges: &[Range<usize>]) {
    cache.retain(|(range, _hash), _| current_ranges.contains(range));
}

/// Extract every `sideromelane://note/...` URL appearing in `text`, in
/// document order, deduplicated. Used to populate
/// [`CachedBlockPreview::link_targets`] without re-scanning the rendered
/// text on every frame.
fn extract_note_link_targets(text: &str) -> Vec<String> {
    let mut targets = Vec::new();
    let mut cursor = 0;
    while let Some(rel) = text[cursor..].find(NOTE_LINK_SCHEME) {
        let start = cursor + rel;
        let after = start + NOTE_LINK_SCHEME.len();
        let end = text[after..]
            .find([')', ' ', '\n', '\r', '\t', '"', '<', '>'])
            .map_or(text.len(), |rel_end| after + rel_end);
        let url = text[start..end].to_owned();
        if !targets.contains(&url) {
            targets.push(url);
        }
        cursor = end;
    }
    targets
}

fn find_wiki_link_prefix(source: &str, cursor_byte: usize) -> Option<&str> {
    let before_cursor = source.get(..cursor_byte)?;
    let open_pos = before_cursor.rfind("[[")?;
    let after_open = &before_cursor[open_pos + 2..];
    if after_open.contains("]]") {
        return None;
    }
    Some(after_open)
}

fn complete_note_links<'a>(stems: &[&'a str], prefix: &str) -> Vec<&'a str> {
    let lower = prefix.to_lowercase();
    stems
        .iter()
        .filter(|s| s.to_lowercase().contains(&lower))
        .copied()
        .take(10)
        .collect()
}

/// Convert a character index to a byte offset in `s`.
fn char_idx_to_byte(s: &str, char_idx: usize) -> usize {
    s.char_indices().nth(char_idx).map_or(s.len(), |(b, _)| b)
}

/// Splice `[[stem]]` into `source`, replacing the `[[prefix` span ending at
/// `cursor_byte`. Returns the new cursor char-position, or `None` if no `[[`
/// precedes `cursor_byte`.
fn splice_wiki_completion(source: &mut String, cursor_byte: usize, stem: &str) -> Option<usize> {
    let open_pos = source[..cursor_byte].rfind("[[")?;
    let completion = format!("[[{stem}]]");
    let open_char = source[..open_pos].chars().count();
    let new_char = open_char + completion.chars().count();
    source.replace_range(open_pos..cursor_byte, &completion);
    Some(new_char)
}

/// Splice `[[stem]]` into `source` and reposition the `TextEdit` cursor after `]]`.
fn apply_wiki_completion(
    ui: &egui::Ui,
    source: &mut String,
    text_edit_id: egui::Id,
    cursor_byte: usize,
    stem: &str,
) {
    let Some(new_char) = splice_wiki_completion(source, cursor_byte, stem) else {
        return;
    };
    if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), text_edit_id) {
        let ccursor = egui::text::CCursor::new(new_char);
        state
            .cursor
            .set_char_range(Some(egui::text::CCursorRange::one(ccursor)));
        egui::TextEdit::store_state(ui.ctx(), text_edit_id, state);
    }
}

/// Keyboard events intercepted from the `InputState` before the `TextEdit`
/// processes them (so the popup owns Enter/Escape/arrows instead).
#[allow(clippy::struct_excessive_bools)]
#[derive(Default)]
struct PopupKeys {
    enter: bool,
    escape: bool,
    down: bool,
    up: bool,
}

/// Consume popup-navigation keys from `InputState` before `TextEdit` processes
/// them. Returns a snapshot of which keys were pressed. When the popup is
/// empty, returns the default (all false) without touching the event queue.
fn collect_popup_keys(ui: &egui::Ui, popup: &WikiLinkPopup) -> PopupKeys {
    if popup.is_empty() {
        return PopupKeys::default();
    }
    ui.input_mut(|i| {
        let keys = PopupKeys {
            enter: i.key_pressed(egui::Key::Enter),
            escape: i.key_pressed(egui::Key::Escape),
            down: i.key_pressed(egui::Key::ArrowDown),
            up: i.key_pressed(egui::Key::ArrowUp),
        };
        i.events.retain(|e| match e {
            egui::Event::Key {
                key, pressed: true, ..
            } => !matches!(
                key,
                egui::Key::Enter | egui::Key::Escape | egui::Key::ArrowDown | egui::Key::ArrowUp
            ),
            _ => true,
        });
        keys
    })
}

/// Register the precomputed `link_targets` from a cached preview with the
/// `CommonMark` cache. Replaces the old per-frame `register_note_links`
/// scan over the rendered text — the targets are already memoised in
/// [`CachedBlockPreview::link_targets`].
fn register_cached_link_targets(cache: &mut CommonMarkCache, targets: &[String]) {
    for url in targets {
        if cache.get_link_hook(url).is_none() {
            cache.add_link_hook(url);
        }
    }
}

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
    folder_index: &FolderIndex,
    popup: &mut WikiLinkPopup,
) -> bool {
    let line_count = note.source.lines().count().max(1);
    let mut changed = false;
    let keys = collect_popup_keys(ui, popup);

    if word_wrap {
        egui::ScrollArea::vertical()
            .id_salt("raw_vscroll")
            .show(ui, |ui| {
                let output = egui::TextEdit::multiline(&mut note.source)
                    .code_editor()
                    .desired_width(ui.available_width())
                    .desired_rows(line_count)
                    .lock_focus(true)
                    .show(ui);
                if let Some(offset) = pending_jump.take() {
                    scroll_text_edit_to_offset(ui, &output.response, offset);
                }
                changed |= output.response.changed();
                raw_popup_pass(
                    ui,
                    &mut note.source,
                    folder_index,
                    popup,
                    &output,
                    &keys,
                    &mut changed,
                );
            });
    } else {
        let mut layouter = nowrap_layouter();
        egui::ScrollArea::both()
            .id_salt("raw_both_scroll")
            .show(ui, |ui| {
                let output = egui::TextEdit::multiline(&mut note.source)
                    .code_editor()
                    .desired_width(f32::INFINITY)
                    .desired_rows(line_count)
                    .lock_focus(true)
                    .layouter(&mut layouter)
                    .show(ui);
                if let Some(offset) = pending_jump.take() {
                    scroll_text_edit_to_offset(ui, &output.response, offset);
                }
                changed |= output.response.changed();
                raw_popup_pass(
                    ui,
                    &mut note.source,
                    folder_index,
                    popup,
                    &output,
                    &keys,
                    &mut changed,
                );
            });
    }

    changed
}

/// Post-TextEdit pass: compute popup items from cursor position and folder
/// index, then show the popup or apply the selected completion.
fn raw_popup_pass(
    ui: &egui::Ui,
    source: &mut String,
    folder_index: &FolderIndex,
    popup: &mut WikiLinkPopup,
    output: &egui::widgets::text_edit::TextEditOutput,
    keys: &PopupKeys,
    changed: &mut bool,
) {
    let Some(char_idx) = output
        .cursor_range
        .and_then(|r| r.single())
        .map(|c| c.index)
    else {
        popup.set_items(vec![]);
        return;
    };
    let cursor_byte = char_idx_to_byte(source, char_idx);

    let Some(prefix) = find_wiki_link_prefix(source, cursor_byte).map(ToOwned::to_owned) else {
        popup.set_items(vec![]);
        return;
    };

    let all_stems: Vec<&str> = folder_index.note_stems().collect();
    let filtered = complete_note_links(&all_stems, &prefix);
    popup.set_items(filtered.iter().map(|&s| s.to_string()).collect());

    if keys.escape {
        popup.set_items(vec![]);
        return;
    }
    if keys.down {
        popup.select_next();
    }
    if keys.up {
        popup.select_prev();
    }
    if keys.enter {
        if let Some(stem) = popup.selected_item() {
            apply_wiki_completion(ui, source, output.response.id, cursor_byte, stem);
            *changed = true;
            popup.set_items(vec![]);
        }
        return;
    }
    if let Some(WikiLinkAction::Selected(stem)) = popup.show(&output.response) {
        apply_wiki_completion(ui, source, output.response.id, cursor_byte, &stem);
        *changed = true;
        popup.set_items(vec![]);
    }
}

/// Position the cursor at `offset` inside the `TextEdit` whose response was
/// just rendered, focus it, and scroll it into view. Lets the outline-jump
/// flow (Spec 0002 AC-1) reach into a `TextEdit`'s state from the outside.
fn scroll_text_edit_to_offset(ui: &egui::Ui, response: &egui::Response, offset: usize) {
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
    cache_bytes: &mut usize,
    block_preview_cache: &mut BlockPreviewCache,
    pending_link_click: &mut Option<String>,
    folder_index: &FolderIndex,
    popup: &mut WikiLinkPopup,
    preview_reading_font: PreviewReadingFont,
) -> bool {
    let blocks = markdown_blocks(&note.source);

    // Sweep stale entries before rendering so the cache stays bounded by the
    // current block layout. This pays for itself across frames because the
    // alternative — recomputing `transform_wiki_links` for every block on
    // every frame — is far costlier.
    let current_ranges: Vec<Range<usize>> = blocks.iter().map(|b| b.range.clone()).collect();
    sweep_stale_block_previews(block_preview_cache, &current_ranges);

    // Resolve a pending jump to a block index before rendering. If the
    // offset falls past the last block (e.g. trailing whitespace beyond all
    // parsed blocks) we clamp to the last block rather than the first, since
    // jumping backwards would surprise the user.
    if let Some(offset) = pending_jump.take() {
        *active_block_index = blocks
            .iter()
            .position(|b| b.range.start <= offset && offset < b.range.end)
            .or_else(|| blocks.len().checked_sub(1));
    }

    let mut changed_block = None;
    let pane_width = ui.available_width();
    let active_block_width = if word_wrap { pane_width } else { f32::INFINITY };

    let note_stem = note.note_id.file_stem().to_owned();

    let scroll_area = if word_wrap {
        egui::ScrollArea::vertical().id_salt("lp_vscroll")
    } else {
        egui::ScrollArea::both().id_salt("lp_both_scroll")
    };
    scroll_area.show(ui, |ui| {
        if !word_wrap {
            ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        }
        // Page navigation when not editing a block.
        if active_block_index.is_none() {
            let page = ui.clip_rect().height();
            if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::PageDown)) {
                ui.scroll_with_delta(egui::vec2(0.0, -page));
            }
            if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::PageUp)) {
                ui.scroll_with_delta(egui::vec2(0.0, page));
            }
        }

        for (index, block) in blocks.iter().enumerate() {
            let is_active = *active_block_index == Some(index);
            let active_fill = ui.visuals().faint_bg_color;
            let frame = if is_active {
                egui::Frame::new().fill(active_fill)
            } else {
                egui::Frame::NONE
            };
            let group_response = frame.show(ui, |ui| {
                if is_active {
                    let mut text = block.text.clone();

                    let keys = collect_popup_keys(ui, popup);

                    let mut layouter = nowrap_layouter();
                    let widget = egui::TextEdit::multiline(&mut text)
                        .code_editor()
                        .desired_width(active_block_width)
                        .desired_rows(block.text.lines().count().max(1));
                    let output = if word_wrap {
                        widget.show(ui)
                    } else {
                        widget.layouter(&mut layouter).show(ui)
                    };

                    let mut completion_changed = false;
                    raw_popup_pass(
                        ui,
                        &mut text,
                        folder_index,
                        popup,
                        &output,
                        &keys,
                        &mut completion_changed,
                    );

                    if output.response.changed() || completion_changed {
                        changed_block = Some((block.range.clone(), text));
                    }
                } else {
                    // Per-block memo: amortise `transform_wiki_links` and the
                    // link-target scan across frames. Cache key combines the
                    // block's byte range with a fast hash of its text so any
                    // edit in the active block invalidates downstream entries
                    // whose ranges shift, and any in-place edit changes the
                    // hash.
                    //
                    // Track the rendered bytes against the soft CommonMark
                    // budget so the caller can reset the cache once the
                    // running total exceeds the cap.
                    *cache_bytes = cache_bytes.saturating_add(block.text.len());
                    let hash = hash_block_text(&block.text);
                    let cached = get_or_insert_cached_preview(
                        block_preview_cache,
                        block.range.clone(),
                        hash,
                        || {
                            let transformed = transform_wiki_links(&block.text, folder_root);
                            let link_targets = extract_note_link_targets(&transformed);
                            CachedBlockPreview {
                                transformed,
                                link_targets,
                            }
                        },
                    );
                    // Register every in-app link so `egui_commonmark` routes clicks
                    // through the cache instead of the OS browser. We feed the
                    // precomputed target list rather than rescanning the rendered text.
                    register_cached_link_targets(cache, &cached.link_targets);

                    // The viewer needs a stable, unique source id per block to keep
                    // its scrollable state. Combine the note stem with the block index.
                    let source_id =
                        egui::Id::new(("sm-mdblock", &note_stem, index, block.range.start));
                    let response = ui
                        .push_id(source_id, |ui| {
                            ui.scope(|ui| {
                                apply_preview_text_style(ui.style_mut(), preview_reading_font);
                                CommonMarkViewer::new().show(ui, cache, &cached.transformed);
                            });
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

#[cfg(test)]
#[allow(clippy::expect_used, clippy::single_range_in_vec_init)]
mod tests {
    use std::cell::Cell;

    use super::{
        BlockPreviewCache, CachedBlockPreview, char_idx_to_byte, complete_note_links,
        extract_note_link_targets, find_wiki_link_prefix, get_or_insert_cached_preview,
        hash_block_text, splice_wiki_completion, sweep_stale_block_previews,
    };

    #[test]
    fn get_or_insert_cached_preview_invokes_compute_once_per_key() {
        let mut cache = BlockPreviewCache::new();
        let calls = Cell::new(0u32);
        let range = 0..10;
        let hash = hash_block_text("body");

        let first = get_or_insert_cached_preview(&mut cache, range.clone(), hash, || {
            calls.set(calls.get() + 1);
            CachedBlockPreview {
                transformed: "out".to_owned(),
                link_targets: Vec::new(),
            }
        });
        assert_eq!(first.transformed, "out");
        assert_eq!(calls.get(), 1, "compute should run on miss");

        let second = get_or_insert_cached_preview(&mut cache, range, hash, || {
            calls.set(calls.get() + 1);
            CachedBlockPreview {
                transformed: "should not be used".to_owned(),
                link_targets: Vec::new(),
            }
        });
        assert_eq!(
            second.transformed, "out",
            "second call must reuse the cached value"
        );
        assert_eq!(calls.get(), 1, "compute must not run on hit");
    }

    #[test]
    fn get_or_insert_cached_preview_misses_on_hash_change() {
        let mut cache = BlockPreviewCache::new();
        let range = 0..10;

        let calls = Cell::new(0u32);
        get_or_insert_cached_preview(&mut cache, range.clone(), 1, || {
            calls.set(calls.get() + 1);
            CachedBlockPreview {
                transformed: String::new(),
                link_targets: Vec::new(),
            }
        });
        get_or_insert_cached_preview(&mut cache, range, 2, || {
            calls.set(calls.get() + 1);
            CachedBlockPreview {
                transformed: String::new(),
                link_targets: Vec::new(),
            }
        });
        assert_eq!(
            calls.get(),
            2,
            "hash change must produce a fresh cache miss"
        );
    }

    #[test]
    fn sweep_stale_block_previews_drops_entries_outside_current_layout() {
        let mut cache = BlockPreviewCache::new();
        cache.insert(
            (0..10, 1),
            CachedBlockPreview {
                transformed: "a".to_owned(),
                link_targets: Vec::new(),
            },
        );
        cache.insert(
            (10..20, 2),
            CachedBlockPreview {
                transformed: "b".to_owned(),
                link_targets: Vec::new(),
            },
        );

        sweep_stale_block_previews(&mut cache, &[0..10]);

        assert!(cache.contains_key(&(0..10, 1)), "0..10 must survive sweep");
        assert!(!cache.contains_key(&(10..20, 2)), "10..20 must be evicted");
    }

    #[test]
    fn extract_note_link_targets_collects_unique_urls() {
        let text = "see [a](sideromelane://note/A) and [b](sideromelane://note/B) and [a again](sideromelane://note/A)";
        let targets = extract_note_link_targets(text);
        assert_eq!(
            targets,
            vec![
                "sideromelane://note/A".to_owned(),
                "sideromelane://note/B".to_owned(),
            ]
        );
    }

    // ── find_wiki_link_prefix ─────────────────────────────────────────────

    #[test]
    fn wiki_prefix_returns_prefix_when_cursor_inside_open_link() {
        assert_eq!(find_wiki_link_prefix("[[Foo", 5), Some("Foo"));
    }

    #[test]
    fn wiki_prefix_returns_empty_string_at_bare_open_bracket() {
        assert_eq!(find_wiki_link_prefix("[[", 2), Some(""));
    }

    #[test]
    fn wiki_prefix_returns_none_when_link_already_closed() {
        assert_eq!(find_wiki_link_prefix("[[Foo]]", 7), None);
    }

    #[test]
    fn wiki_prefix_returns_prefix_for_second_open_link() {
        // [[Done]] [[Bar — cursor at end, second link is open
        let src = "[[Done]] [[Bar";
        assert_eq!(find_wiki_link_prefix(src, src.len()), Some("Bar"));
    }

    #[test]
    fn wiki_prefix_returns_none_when_cursor_before_open_bracket() {
        // cursor is at byte 0, before any `[[`
        assert_eq!(find_wiki_link_prefix("[[Foo", 0), None);
    }

    #[test]
    fn wiki_prefix_returns_none_for_plain_text() {
        let src = "no wiki links here";
        assert_eq!(find_wiki_link_prefix(src, src.len()), None);
    }

    #[test]
    fn wiki_prefix_mid_prefix() {
        // "text [[Foc" — cursor mid-way through the prefix
        let src = "text [[Foc";
        assert_eq!(find_wiki_link_prefix(src, src.len()), Some("Foc"));
    }

    // ── complete_note_links ───────────────────────────────────────────────

    #[test]
    fn complete_links_returns_case_insensitive_substring_matches() {
        let stems = ["Alpha", "alpha-two", "Beta", "Alphabet"];
        assert_eq!(
            complete_note_links(&stems, "alpha"),
            vec!["Alpha", "alpha-two", "Alphabet"]
        );
    }

    #[test]
    fn complete_links_empty_prefix_returns_all_up_to_cap() {
        let stems: Vec<&str> = (0..15).map(|_| "Note").collect();
        assert_eq!(complete_note_links(&stems, "").len(), 10);
    }

    #[test]
    fn complete_links_returns_empty_when_no_match() {
        let stems = ["Alpha", "Beta"];
        assert!(complete_note_links(&stems, "Gamma").is_empty());
    }

    #[test]
    fn complete_links_is_case_insensitive() {
        let stems = ["MyNote"];
        assert_eq!(complete_note_links(&stems, "mynote"), vec!["MyNote"]);
    }

    #[test]
    fn popup_pipeline_filters_by_bracket_prefix() {
        // Simulates: user types "[[Foc" in source, cursor at end.
        let source = "See [[Foc";
        let cursor_byte = source.len();
        let prefix = find_wiki_link_prefix(source, cursor_byte).expect("prefix");
        assert_eq!(prefix, "Foc");
        let stems = ["Focus", "Notes", "Filter"];
        let results = complete_note_links(&stems, prefix);
        assert_eq!(results, vec!["Focus"]);
    }

    #[test]
    fn popup_pipeline_empty_prefix_shows_all_stems() {
        let source = "[[";
        let cursor_byte = source.len();
        let prefix = find_wiki_link_prefix(source, cursor_byte).expect("cursor inside [[ span");
        assert_eq!(prefix, "");
        let stems = ["Alpha", "Beta", "Gamma"];
        let results = complete_note_links(&stems, prefix);
        assert_eq!(results, vec!["Alpha", "Beta", "Gamma"]);
    }

    #[test]
    fn popup_pipeline_completed_link_shows_no_popup() {
        // Once the link is closed with ]], find_wiki_link_prefix returns None.
        let source = "[[Focus]]";
        let cursor_byte = source.len();
        assert!(find_wiki_link_prefix(source, cursor_byte).is_none());
    }

    // ── char_idx_to_byte ─────────────────────────────────────────────────

    #[test]
    fn char_idx_to_byte_ascii_identity() {
        assert_eq!(char_idx_to_byte("hello", 3), 3);
    }

    #[test]
    fn char_idx_to_byte_multibyte_char() {
        // "café": c=0, a=1, f=2, é=3 (2 bytes). Char 4 starts at byte 5.
        assert_eq!(char_idx_to_byte("café", 3), 3);
        assert_eq!(char_idx_to_byte("café", 4), 5);
    }

    #[test]
    fn char_idx_to_byte_past_end_clamps_to_len() {
        assert_eq!(char_idx_to_byte("hi", 99), 2);
    }

    #[test]
    fn char_idx_to_byte_at_zero() {
        assert_eq!(char_idx_to_byte("hello", 0), 0);
    }

    // ── splice_wiki_completion ────────────────────────────────────────────

    #[test]
    fn splice_completes_simple_prefix() {
        let mut source = "text [[Foc".to_string();
        let cursor_byte = source.len();
        let new_char =
            splice_wiki_completion(&mut source, cursor_byte, "Focus").expect("source contains [[");
        assert_eq!(source, "text [[Focus]]");
        assert_eq!(new_char, 14);
    }

    #[test]
    fn splice_completes_at_bare_open_bracket() {
        let mut source = "[[".to_string();
        let cursor_byte = source.len();
        let new_char =
            splice_wiki_completion(&mut source, cursor_byte, "Alpha").expect("source contains [[");
        assert_eq!(source, "[[Alpha]]");
        assert_eq!(new_char, 9);
    }

    #[test]
    fn splice_completes_with_longer_stem_than_prefix() {
        let mut source = "[[A".to_string();
        let cursor_byte = source.len();
        splice_wiki_completion(&mut source, cursor_byte, "Alphabet").expect("source contains [[");
        assert_eq!(source, "[[Alphabet]]");
    }

    #[test]
    fn splice_handles_multibyte_text_before_bracket() {
        // "café [[N" — 'é' is 2 bytes. open_pos is 7 (byte), open_char is 6.
        let mut source = "café [[N".to_string();
        let cursor_byte = source.len();
        let new_char =
            splice_wiki_completion(&mut source, cursor_byte, "Notes").expect("source contains [[");
        assert_eq!(source, "café [[Notes]]");
        // "café " before [[ is 5 chars; completion "[[Notes]]" is 9 chars
        assert_eq!(new_char, 5 + 9);
    }

    #[test]
    fn splice_returns_none_when_no_open_bracket() {
        let mut source = "plain text".to_string();
        let cursor_byte = source.len();
        assert!(splice_wiki_completion(&mut source, cursor_byte, "Note").is_none());
        assert_eq!(source, "plain text");
    }
}
