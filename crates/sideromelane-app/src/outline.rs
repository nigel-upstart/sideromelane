//! Outline-panel rendering helpers.
//!
//! Heading text comes from the core analyzer untouched (raw Markdown source).
//! These helpers strip emphasis markers for display only and supply per-level
//! font sizes so the outline reads as a hierarchy.

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
/// purely a presentation helper.
#[must_use]
pub fn display_heading_text(raw: &str) -> String {
    let mut text = raw.trim_start();
    while let Some(rest) = text.strip_prefix('#') {
        text = rest;
    }
    let mut working = text.trim().to_owned();

    while let Some(stripped) = strip_emphasis_prefix(&working) {
        working = stripped.to_owned();
    }
    while let Some(stripped) = strip_emphasis_suffix(&working) {
        working = stripped.to_owned();
    }

    working.trim().to_owned()
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
