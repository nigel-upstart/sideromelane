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

use crate::NoteRecord;
use crate::preview::{NOTE_LINK_SCHEME, transform_wiki_links};

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
) -> bool {
    if word_wrap {
        let response = ui.add(
            egui::TextEdit::multiline(&mut note.source)
                .code_editor()
                .desired_width(ui.available_width())
                .desired_rows(32)
                .lock_focus(true),
        );
        if let Some(offset) = pending_jump.take() {
            scroll_text_edit_to_offset(ui, &response, offset);
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
            scroll_text_edit_to_offset(ui, &response, offset);
        }
        changed
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
) -> bool {
    let blocks = markdown_blocks(&note.source);

    // Sweep stale entries before rendering so the cache stays bounded by the
    // current block layout. This pays for itself across frames because the
    // alternative — recomputing `transform_wiki_links` for every block on
    // every frame — is far costlier.
    let current_ranges: Vec<Range<usize>> = blocks.iter().map(|b| b.range.clone()).collect();
    sweep_stale_block_previews(block_preview_cache, &current_ranges);

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
                            CommonMarkViewer::new().show(ui, cache, &cached.transformed);
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
        BlockPreviewCache, CachedBlockPreview, extract_note_link_targets,
        get_or_insert_cached_preview, hash_block_text, sweep_stale_block_previews,
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
}
