//! Builds a folder tree from a list of `NoteId`s for the files panel.
//!
//! The tree is built fresh per frame from a sorted note list, so it does not
//! need to be persisted itself. Expand/collapse state is the only persistent
//! piece, and it lives in `FolderSettings::ui::tree_expanded_paths`.

use std::collections::BTreeMap;

use sideromelane_core::NoteId;

/// A directory node in the files-panel tree. The root is implicit; the first
/// level returned by [`build_tree`] is its direct children.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DirNode {
    /// Folder-relative path from the root, using `/` separators.
    pub relative_path: String,
    /// Display name (last path component).
    pub name: String,
    /// Subdirectories of this directory, sorted by name.
    pub subdirs: Vec<Self>,
    /// Note ids whose parent is this directory, sorted by stem.
    pub notes: Vec<NoteId>,
}

/// Builds a directory tree from a flat list of `NoteId`s.
///
/// The returned [`Tree`] holds the root's direct children: subdirectories
/// (sorted by name) and notes that sit at the folder root (sorted by stem).
#[must_use]
pub fn build_tree(notes: &[NoteId]) -> Tree {
    let mut root_dirs: BTreeMap<String, MutableDirNode> = BTreeMap::new();
    let mut root_notes: Vec<NoteId> = Vec::new();

    for note in notes {
        let components = path_components(note.relative_path());
        let Some((_, dir_components)) = components.split_last() else {
            continue;
        };
        if dir_components.is_empty() {
            root_notes.push(note.clone());
        } else {
            insert_path(&mut root_dirs, "", dir_components, note.clone());
        }
    }

    root_notes.sort_by(|left, right| left.file_stem().cmp(right.file_stem()));
    Tree {
        subdirs: root_dirs
            .into_values()
            .map(MutableDirNode::finalize)
            .collect(),
        root_notes,
    }
}

/// The full files-panel tree.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Tree {
    /// Top-level subdirectories, sorted by name.
    pub subdirs: Vec<DirNode>,
    /// Notes that sit at the folder root, sorted by stem.
    pub root_notes: Vec<NoteId>,
}

/// Returns every ancestor directory path (`/`-separated, folder-relative) of
/// the supplied note id. The note's own parent is included; the implicit root
/// is not.
#[must_use]
pub fn ancestor_paths(note: &NoteId) -> Vec<String> {
    let components = path_components(note.relative_path());
    let Some((_, dir_components)) = components.split_last() else {
        return Vec::new();
    };
    (1..=dir_components.len())
        .map(|index| dir_components[..index].join("/"))
        .collect()
}

fn path_components(path: &std::path::Path) -> Vec<String> {
    path.components()
        .filter_map(|component| component.as_os_str().to_str().map(str::to_owned))
        .collect()
}

/// Mutable construction-time mirror of [`DirNode`] using a `BTreeMap` for
/// subdirectories so inserts are O(log N) rather than the drain-and-rebuild
/// dance that a `Vec` would force at every level.
struct MutableDirNode {
    relative_path: String,
    name: String,
    subdirs: BTreeMap<String, Self>,
    notes: Vec<NoteId>,
}

impl MutableDirNode {
    const fn new(name: String, relative_path: String) -> Self {
        Self {
            relative_path,
            name,
            subdirs: BTreeMap::new(),
            notes: Vec::new(),
        }
    }

    fn finalize(self) -> DirNode {
        let mut notes = self.notes;
        notes.sort_by(|left, right| left.file_stem().cmp(right.file_stem()));
        DirNode {
            relative_path: self.relative_path,
            name: self.name,
            subdirs: self.subdirs.into_values().map(Self::finalize).collect(),
            notes,
        }
    }
}

fn insert_path(
    dirs: &mut BTreeMap<String, MutableDirNode>,
    parent_prefix: &str,
    components: &[String],
    note: NoteId,
) {
    let Some((head, rest)) = components.split_first() else {
        return;
    };
    let relative_path = if parent_prefix.is_empty() {
        head.clone()
    } else {
        format!("{parent_prefix}/{head}")
    };
    let entry = dirs
        .entry(head.clone())
        .or_insert_with(|| MutableDirNode::new(head.clone(), relative_path.clone()));

    if rest.is_empty() {
        entry.notes.push(note);
    } else {
        let nested_prefix = entry.relative_path.clone();
        insert_path(&mut entry.subdirs, &nested_prefix, rest, note);
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use std::path::PathBuf;

    use sideromelane_core::NoteId;

    use super::{ancestor_paths, build_tree};

    fn note(path: &str) -> NoteId {
        NoteId::from_folder_relative_path(PathBuf::from(path)).expect("valid note id")
    }

    #[test]
    fn flat_notes_sit_at_root() {
        let tree = build_tree(&[note("a.md"), note("b.md")]);
        assert!(tree.subdirs.is_empty());
        assert_eq!(tree.root_notes.len(), 2);
        assert_eq!(tree.root_notes[0].file_stem(), "a");
        assert_eq!(tree.root_notes[1].file_stem(), "b");
    }

    #[test]
    fn nested_notes_create_subdirs() {
        let tree = build_tree(&[
            note("Cloud/Plan.md"),
            note("Cloud/Charter.md"),
            note("Index.md"),
        ]);
        assert_eq!(tree.root_notes.len(), 1);
        assert_eq!(tree.subdirs.len(), 1);
        assert_eq!(tree.subdirs[0].name, "Cloud");
        assert_eq!(tree.subdirs[0].relative_path, "Cloud");
        assert_eq!(tree.subdirs[0].notes.len(), 2);
        // Sorted by stem.
        assert_eq!(tree.subdirs[0].notes[0].file_stem(), "Charter");
        assert_eq!(tree.subdirs[0].notes[1].file_stem(), "Plan");
    }

    #[test]
    fn deep_nesting_works() {
        let tree = build_tree(&[note("a/b/c/Deep.md")]);
        assert_eq!(tree.subdirs.len(), 1);
        let level1 = &tree.subdirs[0];
        assert_eq!(level1.name, "a");
        assert_eq!(level1.relative_path, "a");
        assert_eq!(level1.subdirs.len(), 1);
        let level2 = &level1.subdirs[0];
        assert_eq!(level2.name, "b");
        assert_eq!(level2.relative_path, "a/b");
        let level3 = &level2.subdirs[0];
        assert_eq!(level3.name, "c");
        assert_eq!(level3.relative_path, "a/b/c");
        assert_eq!(level3.notes.len(), 1);
        assert_eq!(level3.notes[0].file_stem(), "Deep");
    }

    #[test]
    fn directory_with_subdirs_and_notes_keeps_both() {
        let tree = build_tree(&[
            note("Cloud/Plan.md"),
            note("Cloud/Q4/Forecast.md"),
            note("Cloud/Q4/Notes.md"),
            note("Cloud/Charter.md"),
        ]);
        assert_eq!(tree.subdirs.len(), 1);
        let cloud = &tree.subdirs[0];
        assert_eq!(cloud.name, "Cloud");
        // Two notes directly under Cloud, sorted by stem.
        assert_eq!(cloud.notes.len(), 2);
        assert_eq!(cloud.notes[0].file_stem(), "Charter");
        assert_eq!(cloud.notes[1].file_stem(), "Plan");
        // One subdir Q4 with two notes.
        assert_eq!(cloud.subdirs.len(), 1);
        let q4 = &cloud.subdirs[0];
        assert_eq!(q4.name, "Q4");
        assert_eq!(q4.relative_path, "Cloud/Q4");
        assert_eq!(q4.notes.len(), 2);
        assert_eq!(q4.notes[0].file_stem(), "Forecast");
        assert_eq!(q4.notes[1].file_stem(), "Notes");
    }

    #[test]
    fn multiple_top_level_subdirs_are_sorted() {
        let tree = build_tree(&[note("Zeta/A.md"), note("Alpha/B.md"), note("Mid/C.md")]);
        let names: Vec<&str> = tree.subdirs.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, vec!["Alpha", "Mid", "Zeta"]);
    }

    #[test]
    fn ancestor_paths_returns_each_directory_step() {
        let ancestors = ancestor_paths(&note("a/b/c/Deep.md"));
        assert_eq!(ancestors, vec!["a", "a/b", "a/b/c"]);
    }

    #[test]
    fn ancestor_paths_empty_for_root_notes() {
        assert!(ancestor_paths(&note("Index.md")).is_empty());
    }
}
