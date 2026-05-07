//! Outline-panel rendering helpers.
//!
//! Heading text comes from the core analyzer untouched (raw Markdown source).
//! These helpers strip emphasis markers for display only and supply per-level
//! font sizes so the outline reads as a hierarchy.

use sideromelane_core::non_fence_ranges;

fn strip_emphasis_prefix(s: &str) -> Option<&str> {
    for prefix in ["**", "__", "*", "_"] {
        if let Some(stripped) = s.strip_prefix(prefix) {
            return Some(stripped);
        }
    }
    None
}

fn strip_emphasis_suffix(s: &str) -> Option<&str> {
    for suffix in ["**", "__", "*", "_"] {
        if let Some(stripped) = s.strip_suffix(suffix) {
            return Some(stripped);
        }
    }
    None
}

/// Returns the displayable form of a heading line: leading `#` markers and any
/// surrounding `**`/`*`/`__`/`_` emphasis markers stripped.
///
/// Internal whitespace is left alone. The source note is unchanged — this is
/// purely a presentation helper. Walks `&str` slices through every strip step
/// and only allocates once at the end.
#[must_use]
pub fn display_heading_text(raw: &str) -> String {
    let mut text = raw.trim_start().trim_start_matches('#').trim();

    while let Some(stripped) = strip_emphasis_prefix(text) {
        text = stripped;
    }
    while let Some(stripped) = strip_emphasis_suffix(text) {
        text = stripped;
    }

    text.trim().to_owned()
}

/// Returns the outline font size for a Markdown heading level. Heading levels
/// outside 1..=6 are clamped.
#[must_use]
pub fn heading_font_size(level: u8, base: f32) -> f32 {
    let multiplier = match level.clamp(1, 6) {
        1 => 1.25,
        2 => 1.15,
        3 => 1.05,
        4 => 1.00,
        5 => 0.92,
        _ => 0.86,
    };
    base * multiplier
}

/// Returns whether the outline row for this heading level should render bold.
#[must_use]
pub const fn heading_is_bold(level: u8) -> bool {
    level <= 3
}

/// Returns the indent (in pixels) for a heading's outline row.
#[must_use]
pub fn heading_indent_px(level: u8) -> f32 {
    f32::from(level.saturating_sub(1)) * 8.0
}

/// Returns the byte offset in `source` of the first heading line whose parsed
/// level equals `level` and whose display text equals `text` (after applying
/// `display_heading_text`). Lines inside fenced code blocks are skipped.
///
/// Returns `None` if no matching heading is found. Handles `\n` and `\r\n`
/// line endings correctly via `split_inclusive`.
#[must_use]
pub fn byte_offset_for_heading(source: &str, level: u8, text: &str) -> Option<usize> {
    for range in non_fence_ranges(source) {
        let segment = &source[range.clone()];
        let mut offset = range.start;
        for line_with_terminator in segment.split_inclusive('\n') {
            let line = line_with_terminator
                .trim_end_matches('\n')
                .trim_end_matches('\r');
            let line_level = line.bytes().take_while(|&b| b == b'#').count();
            if (1..=6).contains(&line_level)
                && line.as_bytes().get(line_level) == Some(&b' ')
                && u8::try_from(line_level).ok() == Some(level)
                && display_heading_text(line) == text
            {
                return Some(offset);
            }
            offset += line_with_terminator.len();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_hash_prefix() {
        assert_eq!(display_heading_text("# Title"), "Title");
        assert_eq!(display_heading_text("### Deep"), "Deep");
    }

    #[test]
    fn strips_double_star_emphasis() {
        assert_eq!(display_heading_text("## **Bold Heading**"), "Bold Heading");
    }

    #[test]
    fn strips_underscore_emphasis() {
        assert_eq!(display_heading_text("# __Strong__"), "Strong");
        assert_eq!(display_heading_text("# _Italic_"), "Italic");
    }

    #[test]
    fn strips_mixed_markers() {
        assert_eq!(display_heading_text("###  **_Mixed_** "), "Mixed");
    }

    #[test]
    fn passes_plain_text_through() {
        assert_eq!(display_heading_text("# Plain"), "Plain");
    }

    #[test]
    fn handles_only_markers() {
        assert_eq!(display_heading_text("# ****"), "");
    }

    #[test]
    fn handles_only_hashes_and_whitespace() {
        assert_eq!(display_heading_text("###"), "");
        assert_eq!(display_heading_text("##   "), "");
    }

    #[test]
    fn handles_internal_emphasis_runs() {
        // Surrounding markers stripped; internal emphasis kept as-is.
        assert_eq!(
            display_heading_text("## **First** and *second*"),
            "First** and *second"
        );
    }

    #[test]
    fn handles_mismatched_markers() {
        // Leading `***` stripped as `**` then `*`; trailing `*` then nothing.
        assert_eq!(display_heading_text("# ***Bold-ish*"), "Bold-ish");
    }

    #[test]
    fn passes_through_when_no_hash_prefix() {
        // Outline never sees raw lines without `#`, but the helper should be
        // robust if it ever does.
        assert_eq!(display_heading_text("Plain"), "Plain");
    }

    #[test]
    fn byte_offset_simple_match() {
        let source = "# Alpha\n## Beta\n# Gamma\n";
        // "Beta" is an h2, starts at byte 8.
        let offset = byte_offset_for_heading(source, 2, "Beta");
        assert_eq!(offset, Some(8));
    }

    #[test]
    fn byte_offset_skips_fence_fake_heading() {
        let source = "# Real\n```\n# Fake\n```\n";
        // Only the real h1 "Real" should match; the one inside the fence should not.
        assert_eq!(byte_offset_for_heading(source, 1, "Real"), Some(0));
        assert_eq!(byte_offset_for_heading(source, 1, "Fake"), None);
    }

    #[test]
    fn byte_offset_mismatched_level_returns_none() {
        let source = "# Title\n";
        // Correct text but wrong level should return None.
        assert_eq!(byte_offset_for_heading(source, 2, "Title"), None);
    }

    #[test]
    fn byte_offset_first_of_two_duplicates() {
        let source = "# Dup\nsome text\n# Dup\n";
        // Returns the first occurrence at byte 0, not the second.
        assert_eq!(byte_offset_for_heading(source, 1, "Dup"), Some(0));
    }

    #[test]
    fn byte_offset_handles_crlf_line_endings() {
        // CRLF after every line. Offsets must include both \r and \n bytes.
        let source = "# Alpha\r\n## Beta\r\n# Gamma\r\n";
        assert_eq!(byte_offset_for_heading(source, 1, "Alpha"), Some(0));
        // "## Beta" starts after "# Alpha\r\n" which is 9 bytes.
        assert_eq!(byte_offset_for_heading(source, 2, "Beta"), Some(9));
        // "# Gamma" starts after "# Alpha\r\n## Beta\r\n" which is 18 bytes.
        assert_eq!(byte_offset_for_heading(source, 1, "Gamma"), Some(18));
    }

    #[test]
    fn font_sizes_decrease_monotonically() {
        let base = 14.0;
        let sizes: Vec<f32> = (1..=6)
            .map(|level| heading_font_size(level, base))
            .collect();
        for window in sizes.windows(2) {
            assert!(window[0] > window[1], "sizes should strictly decrease");
        }
    }

    #[test]
    fn font_size_clamps_unknown_levels() {
        let base = 14.0;
        let h0 = heading_font_size(0, base);
        let h1 = heading_font_size(1, base);
        let h99 = heading_font_size(99, base);
        let h6 = heading_font_size(6, base);
        assert!((h0 - h1).abs() < f32::EPSILON);
        assert!((h99 - h6).abs() < f32::EPSILON);
    }

    #[test]
    fn indent_grows_with_level() {
        assert!(heading_indent_px(1).abs() < f32::EPSILON);
        assert!(heading_indent_px(3) > heading_indent_px(1));
    }
}
