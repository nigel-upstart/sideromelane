//! Reading typography for rendered Markdown preview blocks.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui::{self, FontData, FontFamily, FontId, TextStyle};

/// Registered egui font family used only by rendered Markdown preview text.
pub const PREVIEW_READING_FAMILY: &str = "sideromelane-preview-reading";
const PREVIEW_READING_FONT: &str = "SideromelanePreviewReading";

/// Body size chosen for long-form reading in the preview pane.
pub const PREVIEW_BODY_FONT_SIZE: f32 = 15.0;
/// Heading anchor size used by `egui_commonmark` when deriving H1-H6 sizes.
pub const PREVIEW_HEADING_FONT_SIZE: f32 = 25.0;
/// Minimum vertical spacing between rendered Markdown elements.
pub const PREVIEW_ITEM_SPACING_Y: f32 = 6.0;

/// Runtime-local system font candidates, ordered by reading preference.
///
/// Georgia is available on stock macOS installs and is designed for screen
/// reading. The Linux fallbacks make headless development/test environments
/// exercise the same code path without bundling a font asset.
const READING_FONT_CANDIDATES: &[&str] = &[
    "/System/Library/Fonts/Supplemental/Georgia.ttf",
    "/Library/Fonts/Georgia.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSerif.ttf",
    "/usr/share/fonts/opentype/urw-base35/NimbusRoman-Regular.otf",
];

/// Availability of the preview reading font family in the current runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PreviewReadingFont {
    /// A local system reading font was registered with egui.
    Registered,
    /// No local reading font candidate was available; use egui proportional text.
    Unavailable,
}

/// Register the preview reading font family if a local system candidate is available.
pub fn install_preview_fonts(ctx: &egui::Context) -> PreviewReadingFont {
    let Some(path) = first_available_reading_font() else {
        return PreviewReadingFont::Unavailable;
    };
    let Ok(bytes) = std::fs::read(path) else {
        return PreviewReadingFont::Unavailable;
    };

    let mut definitions = egui::FontDefinitions::default();
    definitions.font_data.insert(
        PREVIEW_READING_FONT.to_owned(),
        Arc::new(FontData::from_owned(bytes)),
    );
    definitions.families.insert(
        reading_font_family(),
        vec![
            PREVIEW_READING_FONT.to_owned(),
            "Ubuntu-Light".to_owned(),
            "NotoEmoji-Regular".to_owned(),
            "emoji-icon-font".to_owned(),
        ],
    );
    ctx.set_fonts(definitions);
    PreviewReadingFont::Registered
}

/// Apply preview-only reading text styles to an egui style.
pub fn apply_preview_text_style(style: &mut egui::Style, reading_font: PreviewReadingFont) {
    let body_family = match reading_font {
        PreviewReadingFont::Registered => reading_font_family(),
        PreviewReadingFont::Unavailable => FontFamily::Proportional,
    };
    style.text_styles.insert(
        TextStyle::Body,
        FontId::new(PREVIEW_BODY_FONT_SIZE, body_family.clone()),
    );
    style.text_styles.insert(
        TextStyle::Heading,
        FontId::new(PREVIEW_HEADING_FONT_SIZE, body_family),
    );
    style.spacing.item_spacing.y = style.spacing.item_spacing.y.max(PREVIEW_ITEM_SPACING_Y);
}

fn reading_font_family() -> FontFamily {
    FontFamily::Name(Arc::from(PREVIEW_READING_FAMILY))
}

fn first_available_reading_font() -> Option<PathBuf> {
    READING_FONT_CANDIDATES
        .iter()
        .map(Path::new)
        .find(|path| path.is_file())
        .map(Path::to_path_buf)
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use eframe::egui::{FontFamily, Style, TextStyle};

    use super::{
        PREVIEW_BODY_FONT_SIZE, PREVIEW_HEADING_FONT_SIZE, PREVIEW_ITEM_SPACING_Y,
        PREVIEW_READING_FAMILY, PreviewReadingFont, apply_preview_text_style,
    };

    #[test]
    fn preview_style_uses_registered_reading_family_when_available() {
        let mut style = Style::default();

        apply_preview_text_style(&mut style, PreviewReadingFont::Registered);

        let body = style.text_styles.get(&TextStyle::Body).expect("body style");
        assert!((body.size - PREVIEW_BODY_FONT_SIZE).abs() < f32::EPSILON);
        assert_eq!(
            body.family,
            FontFamily::Name(std::sync::Arc::from(PREVIEW_READING_FAMILY))
        );

        let heading = style
            .text_styles
            .get(&TextStyle::Heading)
            .expect("heading style");
        assert!((heading.size - PREVIEW_HEADING_FONT_SIZE).abs() < f32::EPSILON);
        assert_eq!(heading.family, body.family);
        assert!(style.spacing.item_spacing.y >= PREVIEW_ITEM_SPACING_Y);
    }

    #[test]
    fn preview_style_falls_back_to_proportional_when_font_unavailable() {
        let mut style = Style::default();

        apply_preview_text_style(&mut style, PreviewReadingFont::Unavailable);

        let body = style.text_styles.get(&TextStyle::Body).expect("body style");
        assert!((body.size - PREVIEW_BODY_FONT_SIZE).abs() < f32::EPSILON);
        assert_eq!(body.family, FontFamily::Proportional);

        let monospace = style
            .text_styles
            .get(&TextStyle::Monospace)
            .expect("monospace style");
        assert_eq!(monospace.family, FontFamily::Monospace);
    }
}
