//! Core domain types for Sideromelane.

mod analysis;
mod folder;
mod index;
mod note;
pub mod scan;
mod search;

pub use analysis::{Heading, ImageEmbed, NoteAnalysis, WikiLink};
pub use folder::{FolderPathError, NoteId};
pub use index::{Backlink, FolderIndex, Graph, GraphEdge, GraphNode};
pub use note::{Frontmatter, MarkdownNote, MetadataValue};
pub use scan::{ScanError, WalkOptions, walk_markdown_paths};
pub use search::{
    HybridSearchIndex, HybridSearchResult, SearchIndex, SearchQuery, SearchResult,
    SemanticSearchIndex, SemanticSearchResult,
};

/// Returns the current project name.
#[must_use]
pub const fn project_name() -> &'static str {
    "Sideromelane"
}

#[cfg(test)]
mod tests {
    use super::project_name;

    #[test]
    fn project_name_is_stable() {
        assert_eq!(project_name(), "Sideromelane");
    }
}
