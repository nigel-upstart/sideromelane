use crate::{MarkdownNote, MetadataValue, NoteAnalysis, NoteId, merged_tags};

const SEMANTIC_DIMENSIONS: usize = 64;
const SEMANTIC_DIMENSIONS_U64: u64 = 64;
const MIN_SEMANTIC_SCORE: f32 = 0.04;

/// Hybrid lexical and local semantic search index.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HybridSearchIndex {
    lexical_index: SearchIndex,
    semantic_index: SemanticSearchIndex,
}

impl HybridSearchIndex {
    /// Builds a hybrid search index from parsed notes.
    #[must_use]
    pub fn from_notes(notes: impl IntoIterator<Item = MarkdownNote>) -> Self {
        let notes = notes.into_iter().collect::<Vec<_>>();

        Self {
            lexical_index: SearchIndex::from_notes(notes.clone()),
            semantic_index: SemanticSearchIndex::from_notes(notes),
        }
    }

    /// Searches using keyword and local semantic scoring.
    #[must_use]
    pub fn search(&self, query: &SearchQuery) -> Vec<HybridSearchResult> {
        use std::collections::BTreeMap;

        let mut results = BTreeMap::<NoteId, HybridSearchResult>::new();

        for lexical_result in self.lexical_index.search(query) {
            results.insert(
                lexical_result.note_id().clone(),
                HybridSearchResult {
                    note_id: lexical_result.note_id().clone(),
                    keyword_score: lexical_result.score(),
                    semantic_score: 0.0,
                },
            );
        }

        for semantic_result in self.semantic_index.search(query) {
            results
                .entry(semantic_result.note_id().clone())
                .and_modify(|result| result.semantic_score = semantic_result.score())
                .or_insert_with(|| HybridSearchResult {
                    note_id: semantic_result.note_id().clone(),
                    keyword_score: 0,
                    semantic_score: semantic_result.score(),
                });
        }

        let mut results = results.into_values().collect::<Vec<_>>();
        results.sort_by(|left, right| {
            right
                .combined_score()
                .total_cmp(&left.combined_score())
                .then_with(|| left.note_id.cmp(&right.note_id))
        });

        results
    }
}

/// Result from hybrid search.
#[derive(Debug, Clone, PartialEq)]
pub struct HybridSearchResult {
    note_id: NoteId,
    keyword_score: u32,
    semantic_score: f32,
}

impl HybridSearchResult {
    /// Returns the matching note.
    #[must_use]
    pub const fn note_id(&self) -> &NoteId {
        &self.note_id
    }

    /// Returns the lexical score component.
    #[must_use]
    pub const fn keyword_score(&self) -> u32 {
        self.keyword_score
    }

    /// Returns the semantic score component.
    #[must_use]
    pub const fn semantic_score(&self) -> f32 {
        self.semantic_score
    }

    /// Returns the combined deterministic ranking score.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub const fn combined_score(&self) -> f32 {
        self.semantic_score.mul_add(25.0, self.keyword_score as f32)
    }
}

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
            .map(|note| {
                let analysis = NoteAnalysis::from_note(&note);
                SearchDocument::from_note(&note, &analysis)
            })
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

/// Local semantic search index backed by deterministic hashed character vectors.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SemanticSearchIndex {
    documents: Vec<SemanticDocument>,
}

impl SemanticSearchIndex {
    /// Builds a semantic search index from parsed notes.
    #[must_use]
    pub fn from_notes(notes: impl IntoIterator<Item = MarkdownNote>) -> Self {
        let mut documents = notes
            .into_iter()
            .map(|note| SemanticDocument::from_note(&note))
            .collect::<Vec<_>>();

        documents.sort_by(|left, right| left.note_id.cmp(&right.note_id));

        Self { documents }
    }

    /// Searches indexed notes with local vector similarity.
    #[must_use]
    pub fn search(&self, query: &SearchQuery) -> Vec<SemanticSearchResult> {
        let terms = query.terms();
        if terms.is_empty() {
            return Vec::new();
        }

        let query_vector = embed_text(&terms.join(" "));
        let mut results = self
            .documents
            .iter()
            .filter(|document| query.matches_filters(&document.search_document))
            .filter_map(|document| {
                let score = cosine_similarity(&query_vector, &document.embedding);
                (score >= MIN_SEMANTIC_SCORE).then(|| SemanticSearchResult {
                    note_id: document.note_id.clone(),
                    score,
                })
            })
            .collect::<Vec<_>>();

        results.sort_by(|left, right| {
            right
                .score
                .total_cmp(&left.score)
                .then_with(|| left.note_id.cmp(&right.note_id))
        });

        results
    }
}

/// Search result from local semantic matching.
#[derive(Debug, Clone, PartialEq)]
pub struct SemanticSearchResult {
    note_id: NoteId,
    score: f32,
}

impl SemanticSearchResult {
    /// Returns the matching note.
    #[must_use]
    pub const fn note_id(&self) -> &NoteId {
        &self.note_id
    }

    /// Returns cosine similarity in the local embedding space.
    #[must_use]
    pub const fn score(&self) -> f32 {
        self.score
    }
}

#[derive(Debug, Clone, PartialEq)]
struct SemanticDocument {
    note_id: NoteId,
    search_document: SearchDocument,
    embedding: [f32; SEMANTIC_DIMENSIONS],
}

impl SemanticDocument {
    fn from_note(note: &MarkdownNote) -> Self {
        let analysis = NoteAnalysis::from_note(note);
        let search_document = SearchDocument::from_note(note, &analysis);
        let embedding = embed_text(&search_document.embedding_text());

        Self {
            note_id: search_document.note_id.clone(),
            search_document,
            embedding,
        }
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
    fn from_note(note: &MarkdownNote, analysis: &NoteAnalysis) -> Self {
        let note_id = note.note_id().clone();
        let file_name = note.relative_search_path();
        let title = note
            .frontmatter()
            .and_then(|frontmatter| frontmatter.scalar("title"))
            .map(str::to_owned);
        let tags = merged_tags(note, analysis)
            .into_iter()
            .map(|tag| tag.name().to_owned())
            .collect();
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

    fn embedding_text(&self) -> String {
        let mut text = String::new();

        text.push_str(&self.file_name);
        text.push('\n');
        if let Some(title) = &self.title {
            text.push_str(title);
            text.push('\n');
        }
        for tag in &self.tags {
            text.push_str(tag);
            text.push('\n');
        }
        for (key, value) in &self.fields {
            text.push_str(key);
            text.push('\n');
            match value {
                MetadataValue::Scalar(value) => {
                    text.push_str(value);
                    text.push('\n');
                }
                MetadataValue::List(values) => {
                    for value in values {
                        text.push_str(value);
                        text.push('\n');
                    }
                }
            }
        }
        text.push_str(&self.body);

        text
    }
}

trait SearchableNotePath {
    fn relative_search_path(&self) -> String;
}

impl SearchableNotePath for MarkdownNote {
    fn relative_search_path(&self) -> String {
        // NoteId invariant: every path component is valid UTF-8, so `to_str` cannot fail.
        #[allow(clippy::expect_used)]
        self.note_id()
            .relative_path()
            .to_str()
            .expect("NoteId invariant: relative path is always valid UTF-8")
            .to_owned()
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

#[allow(clippy::cast_precision_loss)]
fn embed_text(text: &str) -> [f32; SEMANTIC_DIMENSIONS] {
    let mut vector = [0.0; SEMANTIC_DIMENSIONS];

    for token in normalized_tokens(text) {
        if token.len() <= 3 {
            vector[feature_bucket(&token)] += 1.0;
        } else {
            for feature in char_windows(&token, 3) {
                vector[feature_bucket(&feature)] += 1.0;
            }
        }
    }

    let magnitude = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
    if magnitude > 0.0 {
        for value in &mut vector {
            *value /= magnitude;
        }
    }

    vector
}

fn normalized_tokens(text: &str) -> Vec<String> {
    text.split(|character: char| !character.is_alphanumeric())
        .filter(|token| !token.is_empty())
        .map(str::to_lowercase)
        .collect()
}

fn char_windows(token: &str, size: usize) -> Vec<String> {
    let characters = token.chars().collect::<Vec<_>>();

    characters
        .windows(size)
        .map(|window| window.iter().collect())
        .collect()
}

fn feature_bucket(feature: &str) -> usize {
    let mut hash = 14_695_981_039_346_656_037_u64;

    for byte in feature.bytes() {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(1_099_511_628_211);
    }

    usize::try_from(hash % SEMANTIC_DIMENSIONS_U64).unwrap_or_default()
}

fn cosine_similarity(left: &[f32; SEMANTIC_DIMENSIONS], right: &[f32; SEMANTIC_DIMENSIONS]) -> f32 {
    left.iter()
        .zip(right.iter())
        .map(|(left, right)| left * right)
        .sum()
}
