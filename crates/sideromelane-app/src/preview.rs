//! Live-preview pre-pass: rewrites `Obsidian`-style wiki links and image embeds into
//! standard `CommonMark` links/images so `egui_commonmark` can render them, while
//! leaving fenced code blocks untouched.
//!
//! Substitutions performed on non-fenced regions of the source:
//!
//! - `[[Note]]`               → `[Note](sideromelane://note/Note)`
//! - `[[Note|Alias]]`         → `[Alias](sideromelane://note/Note)`
//! - `[[Note#anchor]]`        → `[Note](sideromelane://note/Note#anchor)`
//! - `[[Note#anchor|Alias]]`  → `[Alias](sideromelane://note/Note#anchor)`
//! - `![[image.png]]`         → `![image](file://<folder_root>/assets/image.png)`
//!
//! The fence scanner mirrors the semantics of `sideromelane_core::analysis::non_fence_ranges`
//! (`CommonMark` fenced code blocks: 0–3 leading spaces, ≥3 of `` ` `` or `~`, closing fence
//! same char, ≥ opening length). Keeping a small local scanner avoids exposing the core
//! helper just for one caller.

use std::ops::Range;
use std::path::Path;

use sideromelane_core::sanitize_asset_filename;

/// Custom URI scheme used for in-app navigation between notes. The path is a percent-
/// encoded note name; `egui_commonmark` will surface link clicks via its hyperlink hook.
pub const NOTE_LINK_SCHEME: &str = "sideromelane://note/";

/// Transforms wiki-style links and image embeds in `source` into `CommonMark` links and
/// images. Content inside fenced code blocks is left untouched.
///
/// `folder_root` is used to resolve image embed paths to absolute `file://` URLs, so the
/// `CommonMark` image renderer can load them regardless of the current working directory.
#[must_use]
pub fn transform_wiki_links(source: &str, folder_root: &Path) -> String {
    let mut out = String::with_capacity(source.len());
    let mut last = 0usize;

    for range in non_fence_ranges(source) {
        // Copy any fenced segment between the last non-fence range and this one verbatim.
        if last < range.start {
            out.push_str(&source[last..range.start]);
        }
        rewrite_segment(&source[range.clone()], folder_root, &mut out);
        last = range.end;
    }

    if last < source.len() {
        out.push_str(&source[last..]);
    }

    out
}

fn rewrite_segment(segment: &str, folder_root: &Path, out: &mut String) {
    let bytes = segment.as_bytes();
    let mut cursor = 0usize;

    while cursor < segment.len() {
        let Some(rel) = segment[cursor..].find("[[") else {
            out.push_str(&segment[cursor..]);
            return;
        };
        let start = cursor + rel;
        let inner_start = start + 2;
        let Some(rel_end) = segment[inner_start..].find("]]") else {
            // Unterminated; treat the rest as literal.
            out.push_str(&segment[cursor..]);
            return;
        };
        let inner_end = inner_start + rel_end;
        let inner = segment[inner_start..inner_end].trim();

        // Detect image embed: a `!` immediately before `[[` with valid prefix.
        let is_image = start > 0
            && bytes.get(start - 1) == Some(&b'!')
            && is_valid_image_embed_prefix(segment, start - 1);

        // Emit text up to the start of the wiki link (or up to the `!` if image embed).
        let copy_end = if is_image { start - 1 } else { start };
        out.push_str(&segment[cursor..copy_end]);

        if inner.is_empty() {
            // Preserve the original literal `[[]]`-ish text.
            out.push_str(&segment[copy_end..inner_end + 2]);
        } else if is_image {
            append_image_embed(inner, folder_root, out);
        } else {
            append_wiki_link(inner, out);
        }

        cursor = inner_end + 2;
    }
}

fn append_wiki_link(inner: &str, out: &mut String) {
    let (target, anchor, alias) = parse_wiki_link_inner(inner);
    if target.is_empty() {
        // Defensive: emit nothing meaningful; reproduce the original.
        out.push_str("[[");
        out.push_str(inner);
        out.push_str("]]");
        return;
    }

    let display = alias.as_deref().unwrap_or(target.as_str());
    out.push('[');
    out.push_str(display);
    out.push_str("](");
    out.push_str(NOTE_LINK_SCHEME);
    out.push_str(&target);
    if let Some(anchor) = anchor.as_deref() {
        out.push('#');
        out.push_str(anchor);
    }
    out.push(')');
}

fn append_image_embed(inner: &str, folder_root: &Path, out: &mut String) {
    // Strip optional alias portion: `![[image.png|Alt]]` is uncommon but tolerated.
    let (target, _anchor, alias) = parse_wiki_link_inner(inner);
    let alt = alias.as_deref().unwrap_or("image");

    // Reject targets that fail filename sanitisation (dot-traversal, bracket
    // injection, etc.). On rejection preserve the literal embed text so the
    // note content is not silently dropped.
    if sanitize_asset_filename(&target).is_err() {
        out.push_str("![[");
        out.push_str(inner);
        out.push_str("]]");
        return;
    }

    let absolute = folder_root.join("assets").join(&target);

    // Canonicalize-and-prefix-check mirrors the H2 image-drop guard in main.rs.
    // If `folder_root` doesn't resolve (e.g. the folder was just picked but not
    // yet created), skip the prefix check and rely on sanitize_asset_filename
    // alone. If both paths resolve, reject if the target escapes the root.
    if let Ok(canonical_root) = folder_root.canonicalize() {
        let canonical_absolute = absolute.canonicalize().unwrap_or_else(|_| {
            // Asset may not exist yet; canonicalize assets/ and rejoin.
            folder_root
                .join("assets")
                .canonicalize()
                .map_or_else(|_| absolute.clone(), |base| base.join(&target))
        });
        if !canonical_absolute.starts_with(&canonical_root) {
            out.push_str("![[");
            out.push_str(inner);
            out.push_str("]]");
            return;
        }
    }

    out.push_str("![");
    out.push_str(alt);
    out.push_str("](file://");
    out.push_str(&absolute.to_string_lossy());
    out.push(')');
}

/// Parse a raw wiki-link inner text into target, optional anchor, optional alias.
/// Mirrors `sideromelane_core::analysis::parse_wiki_link_inner` semantics.
fn parse_wiki_link_inner(inner: &str) -> (String, Option<String>, Option<String>) {
    let (pre_alias, alias) = inner.rfind('|').map_or((inner, None), |pipe| {
        let alias_str = inner[pipe + 1..].trim();
        let alias = if alias_str.is_empty() {
            None
        } else {
            Some(alias_str.to_owned())
        };
        (&inner[..pipe], alias)
    });

    let (target, anchor) = pre_alias.find('#').map_or_else(
        || (pre_alias.trim().to_owned(), None),
        |hash| {
            let anchor_str = pre_alias[hash + 1..].trim();
            let anchor = if anchor_str.is_empty() {
                None
            } else {
                Some(anchor_str.to_owned())
            };
            (pre_alias[..hash].trim().to_owned(), anchor)
        },
    );

    (target, anchor, alias)
}

/// Returns true if the byte immediately before `bang_pos` is a valid image-embed delimiter.
fn is_valid_image_embed_prefix(body: &str, bang_pos: usize) -> bool {
    if bang_pos == 0 {
        return true;
    }
    body[..bang_pos]
        .chars()
        .next_back()
        .is_none_or(|prev| matches!(prev, ' ' | '\t' | '\n' | '\r' | '(' | '[' | '>'))
}

/// Returns byte ranges of `body` that are NOT inside a fenced code block. Mirrors the
/// `CommonMark` rules used by `sideromelane_core::analysis::non_fence_ranges`.
fn non_fence_ranges(body: &str) -> Vec<Range<usize>> {
    let mut ranges = Vec::new();
    let mut segment_start = 0usize;
    let bytes = body.as_bytes();
    let len = body.len();
    let mut pos = 0usize;

    while pos < len {
        let line_start = pos;
        let line_end = bytes[pos..]
            .iter()
            .position(|&b| b == b'\n')
            .map_or(len, |rel| pos + rel + 1);

        let line = &body[line_start..line_end];
        let trimmed = line.trim_end_matches(['\r', '\n']);
        let indent_stripped = trimmed.trim_start_matches(' ');
        let indent = trimmed.len() - indent_stripped.len();

        if indent <= 3 {
            let fence_char = indent_stripped.as_bytes().first().copied();
            if let Some(fc) = fence_char.filter(|&b| matches!(b, b'`' | b'~')) {
                let run = indent_stripped
                    .as_bytes()
                    .iter()
                    .take_while(|&&b| b == fc)
                    .count();
                if run >= 3 {
                    if segment_start < line_start {
                        ranges.push(segment_start..line_start);
                    }

                    let mut inner = line_end;
                    let mut found_close = false;
                    while inner < len {
                        let cl_start = inner;
                        let cl_end = bytes[inner..]
                            .iter()
                            .position(|&b| b == b'\n')
                            .map_or(len, |rel| inner + rel + 1);

                        let cl_line = &body[cl_start..cl_end];
                        let cl_trimmed = cl_line.trim_end_matches(['\r', '\n']);
                        let cl_indent_stripped = cl_trimmed.trim_start_matches(' ');
                        let cl_indent = cl_trimmed.len() - cl_indent_stripped.len();

                        if cl_indent <= 3 {
                            let cl_run = cl_indent_stripped
                                .as_bytes()
                                .iter()
                                .take_while(|&&b| b == fc)
                                .count();
                            if cl_run >= run && cl_indent_stripped[cl_run..].trim().is_empty() {
                                pos = cl_end;
                                segment_start = cl_end;
                                found_close = true;
                                break;
                            }
                        }

                        inner = cl_end;
                    }

                    if !found_close {
                        pos = len;
                        segment_start = len;
                    }
                    continue;
                }
            }
        }

        pos = line_end;
    }

    if segment_start < len {
        ranges.push(segment_start..len);
    }

    ranges
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn root() -> PathBuf {
        PathBuf::from("/notes")
    }

    #[test]
    fn bare_wiki_link_becomes_note_uri() {
        let out = transform_wiki_links("See [[Note]] today.", &root());
        assert_eq!(out, "See [Note](sideromelane://note/Note) today.");
    }

    #[test]
    fn aliased_wiki_link_uses_alias_as_display() {
        let out = transform_wiki_links("See [[Note|Alias]].", &root());
        assert_eq!(out, "See [Alias](sideromelane://note/Note).");
    }

    #[test]
    fn anchored_wiki_link_preserves_anchor() {
        let out = transform_wiki_links("See [[Note#anchor]].", &root());
        assert_eq!(out, "See [Note](sideromelane://note/Note#anchor).");
    }

    #[test]
    fn anchored_aliased_wiki_link_uses_both() {
        let out = transform_wiki_links("See [[Note#anchor|Alias]].", &root());
        assert_eq!(out, "See [Alias](sideromelane://note/Note#anchor).");
    }

    #[test]
    fn image_embed_resolves_against_folder_assets() {
        let out = transform_wiki_links("![[diagram.png]]", &root());
        assert_eq!(out, "![image](file:///notes/assets/diagram.png)");
    }

    #[test]
    fn fenced_code_blocks_are_skipped() {
        let input = "before [[A]]\n\n```\n[[Inside]] should not change\n```\n\nafter [[B]]\n";
        let out = transform_wiki_links(input, &root());
        let expected = "before [A](sideromelane://note/A)\n\n```\n[[Inside]] should not change\n```\n\nafter [B](sideromelane://note/B)\n";
        assert_eq!(out, expected);
    }

    #[test]
    fn tilde_fenced_code_block_is_skipped() {
        let input = "~~~\n[[Inside]]\n~~~\n[[Outside]]\n";
        let out = transform_wiki_links(input, &root());
        assert_eq!(
            out,
            "~~~\n[[Inside]]\n~~~\n[Outside](sideromelane://note/Outside)\n"
        );
    }

    #[test]
    fn empty_brackets_are_left_alone() {
        let out = transform_wiki_links("text [[]] more", &root());
        assert_eq!(out, "text [[]] more");
    }

    #[test]
    fn image_embed_dot_traversal_is_rendered_inert() {
        // `../../../etc/passwd` contains `/` which sanitize_asset_filename rejects.
        let out = transform_wiki_links("![[../../../etc/passwd]]", &root());
        assert_eq!(out, "![[../../../etc/passwd]]");
    }

    #[test]
    fn image_embed_bracket_injected_name_is_rendered_inert() {
        // `[evil]` contains `[` which sanitize_asset_filename rejects.
        let out = transform_wiki_links("![[foo[evil].png]]", &root());
        assert_eq!(out, "![[foo[evil].png]]");
    }

    #[allow(clippy::expect_used)]
    #[test]
    fn image_embed_normal_name_accepted() {
        use std::fs;
        use tempfile::TempDir;
        let dir = TempDir::new().expect("tempdir");
        // Create the assets/ subdirectory so canonicalize can resolve it.
        fs::create_dir(dir.path().join("assets")).expect("create assets dir");
        // With a real directory root the canonicalize check should pass for a
        // well-formed filename.
        let out = transform_wiki_links("![[diagram.png]]", dir.path());
        assert!(
            out.starts_with("![image](file://"),
            "expected file:// url, got: {out}",
        );
        assert!(
            out.contains("diagram.png"),
            "expected filename in url, got: {out}",
        );
    }
}
