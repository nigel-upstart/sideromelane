//! Core domain types for Sideromelane.

mod note;
mod vault;

pub use note::{Frontmatter, MarkdownNote, MetadataValue};
pub use vault::{NoteId, VaultPathError};

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
