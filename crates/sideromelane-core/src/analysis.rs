use crate::MarkdownNote;

/// Extracted note content used by indexing, outlines, backlinks, and graph views.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NoteAnalysis {
    headings: Vec<Heading>,
    wiki_links: Vec<WikiLink>,
    image_embeds: Vec<ImageEmbed>,
}

impl NoteAnalysis {
    /// Extracts analysis data from a parsed note.
    #[must_use]
    pub fn from_note(note: &MarkdownNote) -> Self {
        let headings = extract_headings(note.body());
        let (wiki_links, image_embeds) = extract_wiki_targets(note.body());

        Self {
            headings,
            wiki_links,
            image_embeds,
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

/// Internal note link of the form `[[Note Name]]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WikiLink {
    target: String,
}

impl WikiLink {
    /// Returns the link target inside the wiki-link brackets.
    #[must_use]
    pub fn target(&self) -> &str {
        &self.target
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

fn extract_headings(body: &str) -> Vec<Heading> {
    body.lines().filter_map(parse_heading).collect()
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

fn extract_wiki_targets(body: &str) -> (Vec<WikiLink>, Vec<ImageEmbed>) {
    let mut wiki_links = Vec::new();
    let mut image_embeds = Vec::new();
    let mut cursor = 0;

    while let Some(relative_start) = body[cursor..].find("[[") {
        let start = cursor + relative_start;
        let target_start = start + 2;
        let Some(relative_end) = body[target_start..].find("]]") else {
            break;
        };
        let target_end = target_start + relative_end;
        let target = body[target_start..target_end].trim();

        if !target.is_empty() {
            if body[..start].ends_with('!') {
                image_embeds.push(ImageEmbed {
                    target: target.to_owned(),
                });
            } else {
                wiki_links.push(WikiLink {
                    target: target.to_owned(),
                });
            }
        }

        cursor = target_end + 2;
    }

    (wiki_links, image_embeds)
}
