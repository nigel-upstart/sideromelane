use std::ffi::OsStr;
use std::fmt;
use std::path::{Component, Path, PathBuf};

/// Error returned when a path is not a safe folder-relative note path.
#[derive(Debug, Clone, PartialEq, Eq)]
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
    /// A path component is not valid UTF-8.
    NotUtf8 {
        /// The component that failed UTF-8 round-trip.
        component: PathBuf,
    },
}

impl fmt::Display for FolderPathError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("path is empty"),
            Self::NotRelative => formatter.write_str("path must be relative to the folder"),
            Self::UnsafeComponent => formatter.write_str("path contains an unsafe component"),
            Self::MissingFileName => formatter.write_str("path must include a file name"),
            Self::NotMarkdown => formatter.write_str("path must point to a Markdown note"),
            Self::NotUtf8 { component } => {
                write!(
                    formatter,
                    "path component is not valid UTF-8: {}",
                    component.display()
                )
            }
        }
    }
}

impl std::error::Error for FolderPathError {}

/// Stable identifier for a Markdown note inside a folder.
///
/// Every `NoteId` is guaranteed to have a UTF-8–valid relative path and a UTF-8–valid
/// file stem by construction: `from_folder_relative_path` rejects non-UTF-8 components
/// before producing a `NoteId`. Callers can therefore use `file_stem()` without
/// `unwrap_or_default` fallbacks.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NoteId {
    relative_path: PathBuf,
}

impl NoteId {
    /// Builds a note identifier from a folder-relative Markdown path.
    ///
    /// The path must be relative, must not contain `.` or `..`, must use an `.md`
    /// extension, and every path component must be valid UTF-8.
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

    /// Returns the note file stem as a `&str`.
    ///
    /// This is guaranteed to succeed without fallback because
    /// `from_folder_relative_path` validates UTF-8 for every component.
    #[must_use]
    pub fn file_stem(&self) -> &str {
        // SAFETY: `from_folder_relative_path` validates that every component round-trips
        // through `to_str`, so `file_stem()` and `to_str()` are guaranteed to succeed.
        #[allow(clippy::expect_used)]
        self.relative_path
            .file_stem()
            .and_then(OsStr::to_str)
            .expect("NoteId invariant: file stem is always valid UTF-8")
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
            Component::Normal(os_str) => {
                has_normal_component = true;
                // Reject non-UTF-8 components at the boundary.
                if os_str.to_str().is_none() {
                    return Err(FolderPathError::NotUtf8 {
                        component: PathBuf::from(os_str),
                    });
                }
            }
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
