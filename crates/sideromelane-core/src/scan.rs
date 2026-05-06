//! Folder walker abstraction backed by the `ignore` crate.
//!
//! The walker honors a `.sideromelaneignore` file at the folder root, optionally
//! honors a `.gitignore` file, and defaults to skipping dotfiles, dotfolders, and
//! the app-owned `.sideromelane/` directory.

use std::fmt;
use std::path::{Path, PathBuf};

use ignore::{DirEntry, WalkBuilder};

/// Default maximum depth to defend against pathological folder layouts.
const DEFAULT_MAX_DEPTH: usize = 64;

/// Tunable options for the folder walker.
#[derive(Debug, Clone)]
pub struct WalkOptions {
    /// Whether to follow symbolic links. Defaults to `false` to avoid loops and
    /// unintentional escapes from the folder root.
    pub follow_symlinks: bool,
    /// Whether dotfiles and dotfolders are included. Defaults to `false`.
    pub include_dotfiles: bool,
    /// Whether to honor `.gitignore` files in addition to `.sideromelaneignore`.
    pub honor_gitignore: bool,
    /// Additional ignore filenames the walker should consult.
    pub extra_ignore_files: Vec<PathBuf>,
    /// Maximum directory depth. Defaults to a depth of 64.
    pub max_depth: Option<usize>,
}

impl Default for WalkOptions {
    fn default() -> Self {
        Self {
            follow_symlinks: false,
            include_dotfiles: false,
            honor_gitignore: false,
            extra_ignore_files: Vec::new(),
            max_depth: Some(DEFAULT_MAX_DEPTH),
        }
    }
}

/// Errors surfaced by [`walk_markdown_paths`].
#[derive(Debug)]
pub enum ScanError {
    /// The walker reported an error while traversing the folder.
    Walk(ignore::Error),
}

impl fmt::Display for ScanError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Walk(error) => write!(formatter, "folder walk error: {error}"),
        }
    }
}

impl std::error::Error for ScanError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Walk(error) => Some(error),
        }
    }
}

/// Walks `root` and returns the absolute paths of all Markdown files that pass
/// the configured ignore rules.
///
/// The walker:
/// - Always loads `.sideromelaneignore` from the folder root.
/// - Always excludes the `.sideromelane/` metadata directory itself.
/// - Honors `.gitignore` only when [`WalkOptions::honor_gitignore`] is true.
/// - Skips dotfiles and dotfolders unless [`WalkOptions::include_dotfiles`] is true.
/// - Restricts depth to [`WalkOptions::max_depth`].
/// - Filters to files with a case-insensitive `.md` extension.
pub fn walk_markdown_paths(root: &Path, options: &WalkOptions) -> Result<Vec<PathBuf>, ScanError> {
    let mut builder = WalkBuilder::new(root);
    builder
        .standard_filters(false)
        .hidden(!options.include_dotfiles)
        .follow_links(options.follow_symlinks)
        .max_depth(options.max_depth)
        .add_custom_ignore_filename(".sideromelaneignore");

    if options.honor_gitignore {
        builder.add_custom_ignore_filename(".gitignore");
    }

    for extra in &options.extra_ignore_files {
        builder.add_ignore(extra);
    }

    // Always exclude the app's metadata directory.
    builder.filter_entry(|entry| !is_sideromelane_metadata(entry));

    let mut paths = Vec::new();
    for entry in builder.build() {
        let entry = entry.map_err(ScanError::Walk)?;
        if !entry
            .file_type()
            .is_some_and(|file_type| file_type.is_file())
        {
            continue;
        }
        let path = entry.into_path();
        if has_markdown_extension(&path) {
            paths.push(path);
        }
    }

    paths.sort();
    Ok(paths)
}

fn is_sideromelane_metadata(entry: &DirEntry) -> bool {
    entry.depth() > 0
        && entry
            .file_name()
            .to_str()
            .is_some_and(|name| name == ".sideromelane")
}

fn has_markdown_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("md"))
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::{WalkOptions, walk_markdown_paths};

    fn touch(root: &std::path::Path, relative: &str) {
        let absolute = root.join(relative);
        if let Some(parent) = absolute.parent() {
            fs::create_dir_all(parent).expect("create parent dir");
        }
        fs::write(absolute, b"# note\n").expect("write note");
    }

    fn write(root: &std::path::Path, relative: &str, contents: &str) {
        let absolute = root.join(relative);
        if let Some(parent) = absolute.parent() {
            fs::create_dir_all(parent).expect("create parent dir");
        }
        fs::write(absolute, contents).expect("write file");
    }

    #[test]
    fn defaults_skip_dotfiles_and_metadata() {
        let dir = TempDir::new().expect("tempdir");
        let root = dir.path();
        touch(root, "Note.md");
        touch(root, "subdir/Other.md");
        touch(root, ".hidden/Secret.md");
        touch(root, ".sideromelane/cache.md");
        touch(root, "skip.txt");

        let options = WalkOptions::default();
        let paths = walk_markdown_paths(root, &options).expect("walk ok");
        let names: Vec<String> = paths
            .iter()
            .filter_map(|path| path.strip_prefix(root).ok())
            .map(|path| path.to_string_lossy().into_owned())
            .collect();

        assert!(names.iter().any(|name| name.ends_with("Note.md")));
        assert!(names.iter().any(|name| name.ends_with("Other.md")));
        assert!(!names.iter().any(|name| name.contains(".hidden")));
        assert!(!names.iter().any(|name| name.contains(".sideromelane")));
        assert!(!names.iter().any(|name| name.ends_with("skip.txt")));
    }

    #[test]
    fn include_dotfiles_opt_in() {
        let dir = TempDir::new().expect("tempdir");
        let root = dir.path();
        touch(root, "Note.md");
        touch(root, ".hidden/Secret.md");
        touch(root, ".sideromelane/cache.md");

        let options = WalkOptions {
            include_dotfiles: true,
            ..WalkOptions::default()
        };
        let paths = walk_markdown_paths(root, &options).expect("walk ok");
        let names: Vec<String> = paths
            .iter()
            .filter_map(|path| path.strip_prefix(root).ok())
            .map(|path| path.to_string_lossy().into_owned())
            .collect();

        assert!(names.iter().any(|name| name.contains(".hidden")));
        // `.sideromelane/` is always excluded even with dotfiles enabled.
        assert!(!names.iter().any(|name| name.contains(".sideromelane")));
    }

    #[test]
    fn sideromelaneignore_excludes_paths() {
        let dir = TempDir::new().expect("tempdir");
        let root = dir.path();
        touch(root, "Keep.md");
        touch(root, "Drafts/Skip.md");
        write(root, ".sideromelaneignore", "Drafts/\n");

        let options = WalkOptions::default();
        let paths = walk_markdown_paths(root, &options).expect("walk ok");
        let names: Vec<String> = paths
            .iter()
            .filter_map(|path| path.strip_prefix(root).ok())
            .map(|path| path.to_string_lossy().into_owned())
            .collect();

        assert!(names.iter().any(|name| name.ends_with("Keep.md")));
        assert!(!names.iter().any(|name| name.contains("Drafts")));
    }

    #[test]
    fn gitignore_off_by_default() {
        let dir = TempDir::new().expect("tempdir");
        let root = dir.path();
        touch(root, "Keep.md");
        touch(root, "Build/Skip.md");
        write(root, ".gitignore", "Build/\n");

        let options = WalkOptions::default();
        let paths = walk_markdown_paths(root, &options).expect("walk ok");
        let names: Vec<String> = paths
            .iter()
            .filter_map(|path| path.strip_prefix(root).ok())
            .map(|path| path.to_string_lossy().into_owned())
            .collect();

        // Without honor_gitignore, Build/ is still walked.
        assert!(names.iter().any(|name| name.contains("Build")));

        let opt_in = WalkOptions {
            honor_gitignore: true,
            ..WalkOptions::default()
        };
        let paths = walk_markdown_paths(root, &opt_in).expect("walk ok");
        let names: Vec<String> = paths
            .iter()
            .filter_map(|path| path.strip_prefix(root).ok())
            .map(|path| path.to_string_lossy().into_owned())
            .collect();
        assert!(!names.iter().any(|name| name.contains("Build")));
    }

    #[test]
    fn case_insensitive_md_extension() {
        let dir = TempDir::new().expect("tempdir");
        let root = dir.path();
        touch(root, "Note.MD");
        touch(root, "Other.Md");
        touch(root, "Mixed.md");
        touch(root, "skip.markdown");

        let paths = walk_markdown_paths(root, &WalkOptions::default()).expect("walk ok");
        assert_eq!(paths.len(), 3);
    }
}
