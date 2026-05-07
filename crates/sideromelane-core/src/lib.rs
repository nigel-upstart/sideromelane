//! Core domain types for Sideromelane.

mod analysis;
pub mod asset;
mod folder;
pub mod folder_settings;
mod index;
mod note;
pub mod scan;
mod search;

pub use analysis::{Heading, ImageEmbed, NoteAnalysis, WikiLink, non_fence_ranges};
pub use asset::{
    AssetNameError, ImageMagicError, sanitize_asset_filename, validate_image_magic_bytes,
};
pub use folder::{FolderPathError, NoteId};
pub use folder_settings::{
    FOLDER_METADATA_DIR, FOLDER_SETTINGS_FILE, FolderSettings, FolderSettingsError, IgnoreSettings,
};
pub use index::{Backlink, FolderIndex, Graph, GraphEdge, GraphNode, Neighborhood};
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
