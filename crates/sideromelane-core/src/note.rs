use std::collections::BTreeMap;
use std::ops::Range;

use crate::NoteId;

/// Parsed Markdown note with optional frontmatter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkdownNote {
    note_id: NoteId,
    source: String,
    frontmatter: Option<Frontmatter>,
    body_start: usize,
}

impl MarkdownNote {
    /// Parses a Markdown note from source text.
    #[must_use]
    pub fn parse(note_id: NoteId, source: impl Into<String>) -> Self {
        let source = source.into();

        let Some((frontmatter_range, body_start)) = frontmatter_range(&source) else {
            return Self {
                note_id,
                source,
                frontmatter: None,
                body_start: 0,
            };
        };

        let body_start = skip_one_blank_separator(&source, body_start);
        let frontmatter = Frontmatter::parse(&source[frontmatter_range]);

        Self {
            note_id,
            source,
            frontmatter: Some(frontmatter),
            body_start,
        }
    }

    /// Returns the note identifier.
    #[must_use]
    pub const fn note_id(&self) -> &NoteId {
        &self.note_id
    }

    /// Returns the original note source.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Returns Markdown content after frontmatter.
    #[must_use]
    pub fn body(&self) -> &str {
        &self.source[self.body_start..]
    }

    /// Returns parsed frontmatter, when the source contains a complete frontmatter block.
    #[must_use]
    pub const fn frontmatter(&self) -> Option<&Frontmatter> {
        self.frontmatter.as_ref()
    }
}

/// Frontmatter metadata parsed from a Markdown note.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Frontmatter {
    fields: BTreeMap<String, MetadataValue>,
}

impl Frontmatter {
    fn parse(source: &str) -> Self {
        let fields = source
            .lines()
            .filter_map(parse_frontmatter_line)
            .collect::<BTreeMap<_, _>>();

        Self { fields }
    }

    /// Returns a frontmatter field by key.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&MetadataValue> {
        self.fields.get(key)
    }

    /// Returns a scalar field by key.
    #[must_use]
    pub fn scalar(&self, key: &str) -> Option<&str> {
        match self.get(key) {
            Some(MetadataValue::Scalar(value)) => Some(value.as_str()),
            Some(MetadataValue::List(_)) | None => None,
        }
    }

    /// Returns a list field by key.
    #[must_use]
    pub fn list(&self, key: &str) -> Option<&[String]> {
        match self.get(key) {
            Some(MetadataValue::List(values)) => Some(values),
            Some(MetadataValue::Scalar(_)) | None => None,
        }
    }

    /// Returns whether this frontmatter block contains no parsed fields.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    /// Returns parsed frontmatter fields in deterministic key order.
    pub fn fields(&self) -> impl Iterator<Item = (&str, &MetadataValue)> {
        self.fields.iter().map(|(key, value)| (key.as_str(), value))
    }
}

/// Simple frontmatter value supported by v1 core parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MetadataValue {
    /// Single scalar value.
    Scalar(String),
    /// Inline list value.
    List(Vec<String>),
}

fn frontmatter_range(source: &str) -> Option<(Range<usize>, usize)> {
    let mut lines = source.split_inclusive('\n');
    let first_line = lines.next()?;

    if line_without_newline(first_line).trim() != "---" {
        return None;
    }

    let frontmatter_start = first_line.len();
    let mut cursor = frontmatter_start;

    for line in lines {
        if line_without_newline(line).trim() == "---" {
            return Some((frontmatter_start..cursor, cursor + line.len()));
        }

        cursor += line.len();
    }

    None
}

fn skip_one_blank_separator(source: &str, body_start: usize) -> usize {
    let body = &source[body_start..];

    if body.starts_with("\r\n") {
        body_start + 2
    } else if body.starts_with('\n') {
        body_start + 1
    } else {
        body_start
    }
}

fn parse_frontmatter_line(line: &str) -> Option<(String, MetadataValue)> {
    let line = line.trim();

    if line.is_empty() || line.starts_with('#') {
        return None;
    }

    let (key, value) = line.split_once(':')?;
    let key = key.trim();

    if key.is_empty() {
        return None;
    }

    Some((key.to_owned(), parse_metadata_value(value)))
}

fn parse_metadata_value(value: &str) -> MetadataValue {
    let value = value.trim();

    if let Some(list_value) = value
        .strip_prefix('[')
        .and_then(|inner| inner.strip_suffix(']'))
    {
        return MetadataValue::List(parse_inline_list(list_value));
    }

    MetadataValue::Scalar(strip_matching_quotes(value).to_owned())
}

fn parse_inline_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(strip_matching_quotes)
        .map(str::to_owned)
        .collect()
}

fn strip_matching_quotes(value: &str) -> &str {
    let bytes = value.as_bytes();

    if bytes.len() >= 2
        && ((bytes.first() == Some(&b'"') && bytes.last() == Some(&b'"'))
            || (bytes.first() == Some(&b'\'') && bytes.last() == Some(&b'\'')))
    {
        &value[1..value.len() - 1]
    } else {
        value
    }
}

fn line_without_newline(line: &str) -> &str {
    line.trim_end_matches(['\r', '\n'])
}
