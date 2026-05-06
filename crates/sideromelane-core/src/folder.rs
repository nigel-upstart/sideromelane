use std::ffi::OsStr;
use std::fmt;
use std::path::{Component, Path, PathBuf};

/// Error returned when a path is not a safe folder-relative note path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FolderPathError {
    /// The path is empty.
    Empty,
    /// The path is absolute or contains a platform prefix.
    NotRelative,
    /// The path contains `.` or `..` components.
    UnsafeComponent,
    /// The path does not identify a file name.
    MissingFileName,
    /// The path does not point to a Markdown note.
    NotMarkdown,
}

impl fmt::Display for FolderPathError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Empty => "path is empty",
            Self::NotRelative => "path must be relative to the folder",
            Self::UnsafeComponent => "path contains an unsafe component",
            Self::MissingFileName => "path must include a file name",
            Self::NotMarkdown => "path must point to a Markdown note",
        };

        formatter.write_str(message)
    }
}

impl std::error::Error for FolderPathError {}

/// Stable identifier for a Markdown note inside a folder.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NoteId {
    relative_path: PathBuf,
}

impl NoteId {
    /// Builds a note identifier from a folder-relative Markdown path.
    ///
    /// The path must be relative, must not contain `.` or `..`, and must use an
    /// `.md` extension.
    pub fn from_folder_relative_path(path: impl Into<PathBuf>) -> Result<Self, FolderPathError> {
        let relative_path = path.into();
        validate_folder_note_path(&relative_path)?;

        Ok(Self { relative_path })
    }

    /// Returns the folder-relative path for this note.
    #[must_use]
    pub fn relative_path(&self) -> &Path {
        &self.relative_path
    }

    /// Returns the note file stem.
    #[must_use]
    pub fn file_stem(&self) -> &str {
        self.relative_path
            .file_stem()
            .and_then(OsStr::to_str)
            .unwrap_or_default()
    }
}

fn validate_folder_note_path(path: &Path) -> Result<(), FolderPathError> {
    if path.as_os_str().is_empty() {
        return Err(FolderPathError::Empty);
    }

    if path.is_absolute() {
        return Err(FolderPathError::NotRelative);
    }

    let mut has_normal_component = false;

    for component in path.components() {
        match component {
            Component::Normal(_) => has_normal_component = true,
            Component::CurDir | Component::ParentDir => {
                return Err(FolderPathError::UnsafeComponent);
            }
            Component::Prefix(_) | Component::RootDir => return Err(FolderPathError::NotRelative),
        }
    }

    if !has_normal_component || path.file_name().is_none() {
        return Err(FolderPathError::MissingFileName);
    }

    let is_markdown = path
        .extension()
        .and_then(OsStr::to_str)
        .is_some_and(|extension| extension.eq_ignore_ascii_case("md"));

    if !is_markdown {
        return Err(FolderPathError::NotMarkdown);
    }

    Ok(())
}
