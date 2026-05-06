use crate::{MarkdownNote, MetadataValue, NoteId};

/// In-memory lexical search index for parsed notes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SearchIndex {
    documents: Vec<SearchDocument>,
}

impl SearchIndex {
    /// Builds a search index from parsed notes.
    #[must_use]
    pub fn from_notes(notes: impl IntoIterator<Item = MarkdownNote>) -> Self {
        let mut documents = notes
            .into_iter()
            .map(|note| SearchDocument::from_note(&note))
            .collect::<Vec<_>>();

        documents.sort_by(|left, right| left.note_id.cmp(&right.note_id));

        Self { documents }
    }

    /// Searches indexed notes with deterministic scoring and ordering.
    #[must_use]
    pub fn search(&self, query: &SearchQuery) -> Vec<SearchResult> {
        let terms = query.terms();
        let mut results = self
            .documents
            .iter()
            .filter(|document| query.matches_filters(document))
            .filter_map(|document| document.score(&terms))
            .collect::<Vec<_>>();

        results.sort_by(|left, right| {
            right
                .score
                .cmp(&left.score)
                .then_with(|| left.note_id.cmp(&right.note_id))
        });

        results
    }
}

/// Keyword search query with optional filters.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SearchQuery {
    text: String,
    required_tags: Vec<String>,
    file_name: Option<String>,
    required_fields: Vec<(String, String)>,
}

impl SearchQuery {
    /// Builds an empty query that can be used with filters.
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// Builds a query for lexical text matching.
    #[must_use]
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Self::default()
        }
    }

    /// Requires the note to have the provided tag.
    #[must_use]
    pub fn with_tag(mut self, tag: impl Into<String>) -> Self {
        self.required_tags.push(tag.into());
        self
    }

    /// Requires the note file name or relative path to contain the provided text.
    #[must_use]
    pub fn with_file_name(mut self, file_name: impl Into<String>) -> Self {
        self.file_name = Some(file_name.into());
        self
    }

    /// Requires the note to have a frontmatter field with the provided value.
    #[must_use]
    pub fn with_field(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.required_fields.push((key.into(), value.into()));
        self
    }

    fn terms(&self) -> Vec<String> {
        self.text
            .split_whitespace()
            .map(str::to_lowercase)
            .collect()
    }

    fn matches_filters(&self, document: &SearchDocument) -> bool {
        self.matches_file_name_filter(document)
            && self.matches_tag_filters(document)
            && self.matches_field_filters(document)
    }

    fn matches_file_name_filter(&self, document: &SearchDocument) -> bool {
        self.file_name
            .as_ref()
            .is_none_or(|file_name| contains_normalized(&document.file_name, file_name))
    }

    fn matches_tag_filters(&self, document: &SearchDocument) -> bool {
        self.required_tags.iter().all(|required_tag| {
            document
                .tags
                .iter()
                .any(|tag| equals_normalized(tag, required_tag))
        })
    }

    fn matches_field_filters(&self, document: &SearchDocument) -> bool {
        self.required_fields
            .iter()
            .all(|(key, value)| document.field_matches(key, value))
    }
}

/// Search result for a matching note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResult {
    note_id: NoteId,
    score: u32,
}

impl SearchResult {
    /// Returns the matching note.
    #[must_use]
    pub const fn note_id(&self) -> &NoteId {
        &self.note_id
    }

    /// Returns the deterministic lexical match score.
    #[must_use]
    pub const fn score(&self) -> u32 {
        self.score
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SearchDocument {
    note_id: NoteId,
    file_name: String,
    title: Option<String>,
    tags: Vec<String>,
    fields: Vec<(String, MetadataValue)>,
    body: String,
}

impl SearchDocument {
    fn from_note(note: &MarkdownNote) -> Self {
        let note_id = note.note_id().clone();
        let file_name = note.relative_search_path();
        let title = note
            .frontmatter()
            .and_then(|frontmatter| frontmatter.scalar("title"))
            .map(str::to_owned);
        let tags = note
            .frontmatter()
            .and_then(|frontmatter| frontmatter.list("tags"))
            .map(<[String]>::to_vec)
            .unwrap_or_default();
        let fields = note
            .frontmatter()
            .map(|frontmatter| {
                frontmatter
                    .fields()
                    .map(|(key, value)| (key.to_owned(), value.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let body = note.body().to_owned();

        Self {
            note_id,
            file_name,
            title,
            tags,
            fields,
            body,
        }
    }

    fn score(&self, terms: &[String]) -> Option<SearchResult> {
        if terms.is_empty() {
            return Some(SearchResult {
                note_id: self.note_id.clone(),
                score: 1,
            });
        }

        let score = terms.iter().map(|term| self.score_term(term)).sum::<u32>();

        (score > 0).then(|| SearchResult {
            note_id: self.note_id.clone(),
            score,
        })
    }

    fn score_term(&self, term: &str) -> u32 {
        let mut score = 0;

        if self
            .title
            .as_ref()
            .is_some_and(|title| contains_normalized(title, term))
        {
            score += 50;
        }

        if contains_normalized(&self.file_name, term) {
            score += 40;
        }

        if self.tags.iter().any(|tag| contains_normalized(tag, term)) {
            score += 30;
        }

        if self
            .fields
            .iter()
            .any(|(key, value)| field_contains(key, value, term))
        {
            score += 20;
        }

        if contains_normalized(&self.body, term) {
            score += 10;
        }

        score
    }

    fn field_matches(&self, required_key: &str, required_value: &str) -> bool {
        self.fields.iter().any(|(key, value)| {
            equals_normalized(key, required_key) && value_matches(value, required_value)
        })
    }
}

trait SearchableNotePath {
    fn relative_search_path(&self) -> String;
}

impl SearchableNotePath for MarkdownNote {
    fn relative_search_path(&self) -> String {
        self.note_id()
            .relative_path()
            .to_string_lossy()
            .into_owned()
    }
}

fn field_contains(key: &str, value: &MetadataValue, term: &str) -> bool {
    contains_normalized(key, term)
        || match value {
            MetadataValue::Scalar(value) => contains_normalized(value, term),
            MetadataValue::List(values) => {
                values.iter().any(|value| contains_normalized(value, term))
            }
        }
}

fn value_matches(value: &MetadataValue, required_value: &str) -> bool {
    match value {
        MetadataValue::Scalar(value) => equals_normalized(value, required_value),
        MetadataValue::List(values) => values
            .iter()
            .any(|value| equals_normalized(value, required_value)),
    }
}

fn contains_normalized(value: &str, needle: &str) -> bool {
    value.to_lowercase().contains(&needle.to_lowercase())
}

fn equals_normalized(left: &str, right: &str) -> bool {
    left.to_lowercase() == right.to_lowercase()
}
