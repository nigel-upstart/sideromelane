use crate::MarkdownNote;
use crate::note::Tag;

/// Extracted note content used by indexing, outlines, backlinks, and graph views.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NoteAnalysis {
    headings: Vec<Heading>,
    wiki_links: Vec<WikiLink>,
    image_embeds: Vec<ImageEmbed>,
    inline_tags: Vec<Tag>,
}

impl NoteAnalysis {
    /// Extracts analysis data from a parsed note.
    #[must_use]
    pub fn from_note(note: &MarkdownNote) -> Self {
        let headings = extract_headings(note.body());
        let (wiki_links, image_embeds) = extract_wiki_targets(note.body());
        let inline_tags = extract_inline_tags(note.body());

        Self {
            headings,
            wiki_links,
            image_embeds,
            inline_tags,
        }
    }

    /// Returns Markdown headings found in note body order.
    #[must_use]
    pub fn headings(&self) -> &[Heading] {
        &self.headings
    }

    /// Returns non-image wiki links found in note body order.
    #[must_use]
    pub fn wiki_links(&self) -> &[WikiLink] {
        &self.wiki_links
    }

    /// Returns wiki-style image embeds found in note body order.
    #[must_use]
    pub fn image_embeds(&self) -> &[ImageEmbed] {
        &self.image_embeds
    }

    /// Returns inline `#tag` mentions found in note body order, deduplicated.
    #[must_use]
    pub fn inline_tags(&self) -> &[Tag] {
        &self.inline_tags
    }
}

/// Returns the deduplicated, sorted union of frontmatter and inline tags for a note.
///
/// This is the single source of truth for downstream consumers; nothing should
/// read frontmatter tags and inline tags independently.
#[must_use]
pub fn merged_tags(note: &MarkdownNote, analysis: &NoteAnalysis) -> Vec<Tag> {
    use std::collections::BTreeSet;

    let mut tags = BTreeSet::new();

    if let Some(frontmatter) = note.frontmatter() {
        for tag in frontmatter.tags() {
            tags.insert(tag);
        }
    }

    for tag in analysis.inline_tags() {
        tags.insert(tag.clone());
    }

    tags.into_iter().collect()
}

/// Markdown heading extracted from a note body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Heading {
    level: u8,
    text: String,
}

impl Heading {
    /// Returns the Markdown heading level from 1 through 6.
    #[must_use]
    pub const fn level(&self) -> u8 {
        self.level
    }

    /// Returns the heading text without leading hash markers.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }
}

/// Internal note link of the form `[[Note Name]]`, optionally with anchor and/or alias.
///
/// Syntax variants supported:
/// - `[[Target]]` — bare link
/// - `[[Target#anchor]]` — link with anchor
/// - `[[Target|alias]]` — link with display alias
/// - `[[Target#anchor|alias]]` — link with both
///
/// Resolution uses `target` only; `alias` is purely for display. `anchor` is preserved
/// for future deep-linking but is not used in v1 resolution. See ADR 0006.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WikiLink {
    target: String,
    anchor: Option<String>,
    alias: Option<String>,
}

impl WikiLink {
    /// Returns the link target (the part before `#` and `|`).
    #[must_use]
    pub fn target(&self) -> &str {
        &self.target
    }

    /// Returns the anchor fragment, if present (the part after `#` and before `|`).
    #[must_use]
    pub fn anchor(&self) -> Option<&str> {
        self.anchor.as_deref()
    }

    /// Returns the display alias, if present (the part after `|`).
    #[must_use]
    pub fn alias(&self) -> Option<&str> {
        self.alias.as_deref()
    }
}

/// Wiki-style image embed of the form `![[image.png]]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageEmbed {
    target: String,
}

impl ImageEmbed {
    /// Returns the image target inside the wiki-link brackets.
    #[must_use]
    pub fn target(&self) -> &str {
        &self.target
    }
}

/// Returns byte ranges of `body` that are NOT inside a fenced code block.
///
/// Recognises backtick (` ``` `) and tilde (`~~~`) fences per the `CommonMark` rules:
/// - Opening fence: at least 3 of the same character (backtick or tilde), indented 0-3 spaces.
/// - Closing fence: same character, at least as long as the opening, indented 0-3 spaces.
/// - Content between fences is excluded from the returned ranges.
///
/// Exposed so that app-side helpers (e.g. the outline jump) can skip fenced
/// regions when scanning for heading byte offsets without duplicating this logic.
#[must_use]
pub fn non_fence_ranges(body: &str) -> Vec<std::ops::Range<usize>> {
    let mut ranges = Vec::new();
    let mut segment_start = 0usize;
    let bytes = body.as_bytes();
    let len = body.len();
    let mut pos = 0usize;

    while pos < len {
        // Find the next line start from pos.
        let line_start = pos;

        // Find end of this line.
        let line_end = bytes[pos..]
            .iter()
            .position(|&b| b == b'\n')
            .map_or(len, |rel| pos + rel + 1);

        let line = &body[line_start..line_end];
        let trimmed = line.trim_end_matches(['\r', '\n']);

        // Strip 0–3 leading spaces (indent allowed by CommonMark).
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
                    // This is an opening fence. Push non-fence range up to here.
                    if segment_start < line_start {
                        ranges.push(segment_start..line_start);
                    }

                    // Scan forward for the matching closing fence.
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
                            // Closing fence: same char, ≥ opening length, rest is whitespace.
                            if cl_run >= run && cl_indent_stripped[cl_run..].trim().is_empty() {
                                // Skip past the closing fence line.
                                pos = cl_end;
                                segment_start = cl_end;
                                found_close = true;
                                break;
                            }
                        }

                        inner = cl_end;
                    }

                    if !found_close {
                        // Unclosed fence — treat rest of body as fenced (don't extract).
                        pos = len;
                        segment_start = len;
                    }
                    continue;
                }
            }
        }

        pos = line_end;
    }

    // Trailing non-fence segment.
    if segment_start < len {
        ranges.push(segment_start..len);
    }

    ranges
}

/// Returns byte ranges of `body` that are outside fenced code blocks AND inline
/// code spans (single or multi-backtick). Used by [`extract_inline_tags`] to
/// skip both fenced blocks and inline code.
fn non_code_ranges(body: &str) -> Vec<std::ops::Range<usize>> {
    let body_bytes = body.as_bytes();
    let mut result = Vec::new();

    for range in non_fence_ranges(body) {
        let offset = range.start;
        let seg_bytes = &body_bytes[range];
        let mut seg_start = 0usize;
        let mut i = 0usize;

        while i < seg_bytes.len() {
            if seg_bytes[i] != b'`' {
                i += 1;
                continue;
            }

            let tick_start = i;
            let tick_len = seg_bytes[tick_start..]
                .iter()
                .take_while(|&&b| b == b'`')
                .count();

            // Search for a matching closing run of the same backtick length.
            let mut search = tick_start + tick_len;
            let mut found_close = false;

            while search + tick_len <= seg_bytes.len() {
                if seg_bytes[search] == b'`' {
                    let close_run = seg_bytes[search..]
                        .iter()
                        .take_while(|&&b| b == b'`')
                        .count();
                    if close_run == tick_len {
                        if seg_start < tick_start {
                            result.push(offset + seg_start..offset + tick_start);
                        }
                        i = search + tick_len;
                        seg_start = i;
                        found_close = true;
                        break;
                    }
                    // Different-length run — skip it.
                    search += close_run;
                } else {
                    search += 1;
                }
            }

            if !found_close {
                // No matching close — treat the backtick run as literal text.
                i = tick_start + tick_len;
            }
        }

        if seg_start < seg_bytes.len() {
            result.push(offset + seg_start..offset + seg_bytes.len());
        }
    }

    result
}

/// Returns inline `#tag` mentions from `body` in document order, deduplicated.
///
/// Tags inside fenced code blocks and inline code spans are skipped. A `#` is
/// only treated as a tag opener when it is at the start of the body or preceded
/// by whitespace — `text#middle` is not a tag and `# Heading` (space after `#`)
/// is not a tag. Trailing punctuation (`.`, `,`, `;`, `:`, `!`, `?`, `)`, `]`,
/// `}`, `"`, `'`) is stripped from the tag name.
#[must_use]
pub fn extract_inline_tags(body: &str) -> Vec<Tag> {
    use std::collections::BTreeSet;

    let body_bytes = body.as_bytes();
    let mut seen = BTreeSet::new();
    let mut result = Vec::new();

    for range in non_code_ranges(body) {
        let seg = &body[range.clone()];
        let seg_bytes = seg.as_bytes();
        let offset = range.start;
        let mut i = 0usize;

        while i < seg_bytes.len() {
            if seg_bytes[i] != b'#' {
                i += 1;
                continue;
            }

            let abs = offset + i;
            let preceded_by_boundary =
                abs == 0 || matches!(body_bytes[abs - 1], b' ' | b'\t' | b'\n' | b'\r');

            if !preceded_by_boundary {
                i += 1;
                continue;
            }

            let tag_start = i + 1;
            let tag_len = seg_bytes[tag_start..]
                .iter()
                .take_while(|&&b| is_tag_byte(b))
                .count();

            if tag_len == 0 {
                // `# ` or bare `#` — not a tag.
                i += 1;
                continue;
            }

            // Strip trailing punctuation.
            let raw_end = tag_start + tag_len;
            let trailing = seg_bytes[tag_start..raw_end]
                .iter()
                .rev()
                .take_while(|&&b| is_trailing_punct_byte(b))
                .count();
            let tag_end = raw_end - trailing;

            if tag_end > tag_start
                && let Ok(tag) = Tag::new(&seg[tag_start..tag_end])
                && !seen.contains(&tag)
            {
                seen.insert(tag.clone());
                result.push(tag);
            }

            i = raw_end;
        }
    }

    result
}

const fn is_tag_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'/')
}

const fn is_trailing_punct_byte(b: u8) -> bool {
    matches!(
        b,
        b'.' | b',' | b';' | b':' | b'!' | b'?' | b')' | b']' | b'}' | b'"' | b'\''
    )
}

fn extract_headings(body: &str) -> Vec<Heading> {
    non_fence_ranges(body)
        .into_iter()
        .flat_map(|range| {
            body[range]
                .lines()
                .filter_map(parse_heading)
                .collect::<Vec<_>>()
        })
        .collect()
}

fn parse_heading(line: &str) -> Option<Heading> {
    let level = line.bytes().take_while(|byte| *byte == b'#').count();

    if !(1..=6).contains(&level) || line.as_bytes().get(level) != Some(&b' ') {
        return None;
    }

    let text = line[level + 1..].trim();

    if text.is_empty() {
        return None;
    }

    Some(Heading {
        level: u8::try_from(level).ok()?,
        text: text.to_owned(),
    })
}

/// Returns true if the byte immediately before the `!` in `body[..bang_pos]` is a valid
/// image-embed delimiter: start-of-string, whitespace, `(`, `[`, or `>`.
///
/// This prevents `text![[…]]` or `!![[…]]` from being classified as image embeds.
fn is_valid_image_embed_prefix(body: &str, bang_pos: usize) -> bool {
    if bang_pos == 0 {
        return true;
    }
    // Walk back one Unicode char.
    body[..bang_pos]
        .chars()
        .next_back()
        .is_none_or(|prev| matches!(prev, ' ' | '\t' | '\n' | '\r' | '(' | '[' | '>'))
}

/// Parse a raw wiki-link inner text into target, optional anchor, optional alias.
///
/// Formats: `Target`, `Target#anchor`, `Target|alias`, `Target#anchor|alias`.
fn parse_wiki_link_inner(inner: &str) -> (String, Option<String>, Option<String>) {
    // Split off alias first (everything after the last `|`).
    let (pre_alias, alias) = inner.rfind('|').map_or((inner, None), |pipe| {
        let alias_str = inner[pipe + 1..].trim();
        let alias = if alias_str.is_empty() {
            None
        } else {
            Some(alias_str.to_owned())
        };
        (&inner[..pipe], alias)
    });

    // Split anchor from target.
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

fn extract_wiki_targets(body: &str) -> (Vec<WikiLink>, Vec<ImageEmbed>) {
    let mut wiki_links = Vec::new();
    let mut image_embeds = Vec::new();

    for range in non_fence_ranges(body) {
        let segment = &body[range.clone()];
        let offset = range.start;
        let mut cursor = 0usize;

        while let Some(relative_start) = segment[cursor..].find("[[") {
            let start = cursor + relative_start;
            let target_start = start + 2;
            let Some(relative_end) = segment[target_start..].find("]]") else {
                break;
            };
            let target_end = target_start + relative_end;
            let inner = segment[target_start..target_end].trim();

            if !inner.is_empty() {
                // Check if preceded by `!` for image embed.
                let abs_start = offset + start;
                let is_image = abs_start > 0
                    && body.as_bytes().get(abs_start - 1) == Some(&b'!')
                    && is_valid_image_embed_prefix(body, abs_start - 1);

                if is_image {
                    image_embeds.push(ImageEmbed {
                        target: inner.to_owned(),
                    });
                } else {
                    let (target, anchor, alias) = parse_wiki_link_inner(inner);
                    if !target.is_empty() {
                        wiki_links.push(WikiLink {
                            target,
                            anchor,
                            alias,
                        });
                    }
                }
            }

            cursor = target_end + 2;
        }
    }

    (wiki_links, image_embeds)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::note::{Tag, TagError};

    fn tag(s: &str) -> Tag {
        Tag::new(s).expect("valid tag in test fixture")
    }

    // --- Tag::new validation ---

    #[test]
    fn tag_new_strips_leading_hash() {
        assert_eq!(Tag::new("#foo").expect("valid").name(), "foo");
        assert_eq!(Tag::new("foo").expect("valid").name(), "foo");
    }

    #[test]
    fn tag_new_round_trip() {
        assert_eq!(Tag::new("#foo"), Tag::new("foo"));
    }

    #[test]
    fn tag_new_rejects_empty() {
        assert_eq!(Tag::new(""), Err(TagError::Empty));
        assert_eq!(Tag::new("#"), Err(TagError::Empty));
    }

    #[test]
    fn tag_new_rejects_invalid_chars() {
        assert!(matches!(
            Tag::new("foo bar"),
            Err(TagError::InvalidChar(' '))
        ));
        assert!(matches!(
            Tag::new("foo#bar"),
            Err(TagError::InvalidChar('#'))
        ));
    }

    #[test]
    fn tag_new_accepts_nested() {
        assert_eq!(
            Tag::new("kubernetes/storage").expect("valid").name(),
            "kubernetes/storage"
        );
        assert_eq!(
            Tag::new("#kubernetes/storage").expect("valid").name(),
            "kubernetes/storage"
        );
    }

    #[test]
    fn tag_new_accepts_hyphen_underscore() {
        assert!(Tag::new("my-tag_v2").is_ok());
    }

    // --- extract_inline_tags ---

    #[test]
    fn extracts_simple_tag() {
        let tags = extract_inline_tags("#foo");
        assert_eq!(tags, vec![tag("foo")]);
    }

    #[test]
    fn extracts_tag_after_whitespace() {
        let tags = extract_inline_tags("hello #world");
        assert_eq!(tags, vec![tag("world")]);
    }

    #[test]
    fn skips_mid_word_hash() {
        let tags = extract_inline_tags("text#middle");
        assert!(tags.is_empty());
    }

    #[test]
    fn skips_heading_style() {
        // `# Heading` — space after `#` means zero tag chars follow
        let tags = extract_inline_tags("# Heading\n## Another");
        assert!(tags.is_empty());
    }

    #[test]
    fn strips_trailing_punctuation() {
        let tags = extract_inline_tags("#tag. #tag, #tag; #tag: #tag!");
        assert_eq!(tags, vec![tag("tag")]);
    }

    #[test]
    fn strips_trailing_brackets() {
        assert_eq!(extract_inline_tags("#tag)"), vec![tag("tag")]);
        assert_eq!(extract_inline_tags("#tag]"), vec![tag("tag")]);
        assert_eq!(extract_inline_tags("#tag}"), vec![tag("tag")]);
    }

    #[test]
    fn deduplicates_tags() {
        let tags = extract_inline_tags("#foo #bar #foo");
        assert_eq!(tags, vec![tag("foo"), tag("bar")]);
    }

    #[test]
    fn handles_nested_tags() {
        let tags = extract_inline_tags("#kubernetes/storage");
        assert_eq!(tags, vec![tag("kubernetes/storage")]);
    }

    #[test]
    fn multiple_tags_one_line() {
        let tags = extract_inline_tags("#a #b #c");
        assert_eq!(tags, vec![tag("a"), tag("b"), tag("c")]);
    }

    #[test]
    fn tag_at_start_of_string() {
        let tags = extract_inline_tags("#first body text");
        assert_eq!(tags, vec![tag("first")]);
    }

    #[test]
    fn tag_after_newline() {
        let tags = extract_inline_tags("line one\n#tag");
        assert_eq!(tags, vec![tag("tag")]);
    }

    #[test]
    fn skips_tags_in_fenced_code() {
        let body = "text\n```\n#not-a-tag\n```\n#real-tag";
        let tags = extract_inline_tags(body);
        assert_eq!(tags, vec![tag("real-tag")]);
    }

    #[test]
    fn skips_tags_in_inline_code() {
        let body = "see `#not-a-tag` and #real-tag";
        let tags = extract_inline_tags(body);
        assert_eq!(tags, vec![tag("real-tag")]);
    }

    #[test]
    fn skips_tags_in_multi_backtick_code() {
        let body = "see ``#not-a-tag`` and #real-tag";
        let tags = extract_inline_tags(body);
        assert_eq!(tags, vec![tag("real-tag")]);
    }

    #[test]
    fn mismatched_backtick_run_is_not_a_code_span() {
        // Opener tick_len=1, no valid closer (mid-run has len=2): not a code span.
        // Text is all literal; #real (preceded by space) must be extracted.
        let tags = extract_inline_tags("`foo `` #real");
        assert_eq!(tags, vec![tag("real")]);
    }

    #[test]
    fn longer_inner_run_does_not_close_code_span() {
        // "`foo```bar`" — opener 1, inner run 3 (not a closer), final 1 (valid closer).
        // The whole span is code; #tag inside should be skipped.
        let tags = extract_inline_tags("`foo```#skipped` #real");
        assert_eq!(tags, vec![tag("real")]);
    }
}
