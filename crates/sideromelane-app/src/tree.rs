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
/// The returned vector is the root's direct children: subdirectories first,
/// then notes that live at the folder root (those go into [`Tree::root_notes`]
/// in [`build_tree_full`]).
#[must_use]
pub fn build_tree(notes: &[NoteId]) -> Tree {
    let mut root_dirs: BTreeMap<String, DirNode> = BTreeMap::new();
    let mut root_notes: Vec<NoteId> = Vec::new();

    for note in notes {
        let components: Vec<String> = note
            .relative_path()
            .components()
            .filter_map(|component| component.as_os_str().to_str().map(str::to_owned))
            .collect();
        if components.len() <= 1 {
            root_notes.push(note.clone());
            continue;
        }
        let dir_components = &components[..components.len() - 1];
        insert_dir(&mut root_dirs, dir_components, note.clone());
    }

    Tree {
        subdirs: root_dirs.into_values().map(finalize).collect(),
        root_notes: {
            let mut sorted = root_notes;
            sorted.sort_by(|left, right| left.file_stem().cmp(right.file_stem()));
            sorted
        },
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
    let components: Vec<String> = note
        .relative_path()
        .components()
        .filter_map(|component| component.as_os_str().to_str().map(str::to_owned))
        .collect();
    if components.len() <= 1 {
        return Vec::new();
    }

    let mut ancestors = Vec::new();
    let dir_components = &components[..components.len() - 1];
    for index in 1..=dir_components.len() {
        ancestors.push(dir_components[..index].join("/"));
    }
    ancestors
}

fn insert_dir(dirs: &mut BTreeMap<String, DirNode>, components: &[String], note: NoteId) {
    let Some((head, rest)) = components.split_first() else {
        return;
    };
    let entry = dirs.entry(head.clone()).or_insert_with(|| DirNode {
        relative_path: head.clone(),
        name: head.clone(),
        subdirs: Vec::new(),
        notes: Vec::new(),
    });

    if rest.is_empty() {
        entry.notes.push(note);
    } else {
        let mut nested: BTreeMap<String, DirNode> = entry
            .subdirs
            .drain(..)
            .map(|node| (node.name.clone(), node))
            .collect();
        let mut child_components = Vec::with_capacity(rest.len());
        let parent_prefix = entry.relative_path.clone();
        for component in rest {
            child_components.push(component.clone());
        }
        insert_into_nested(&mut nested, &parent_prefix, &child_components, note);
        entry.subdirs = nested.into_values().collect();
    }
}

fn insert_into_nested(
    dirs: &mut BTreeMap<String, DirNode>,
    parent_prefix: &str,
    components: &[String],
    note: NoteId,
) {
    let Some((head, rest)) = components.split_first() else {
        return;
    };
    let relative_path = format!("{parent_prefix}/{head}");
    let entry = dirs.entry(head.clone()).or_insert_with(|| DirNode {
        relative_path: relative_path.clone(),
        name: head.clone(),
        subdirs: Vec::new(),
        notes: Vec::new(),
    });

    if rest.is_empty() {
        entry.notes.push(note);
    } else {
        let mut nested: BTreeMap<String, DirNode> = entry
            .subdirs
            .drain(..)
            .map(|node| (node.name.clone(), node))
            .collect();
        let nested_parent = entry.relative_path.clone();
        insert_into_nested(&mut nested, &nested_parent, rest, note);
        entry.subdirs = nested.into_values().collect();
    }
}

fn finalize(mut node: DirNode) -> DirNode {
    node.notes
        .sort_by(|left, right| left.file_stem().cmp(right.file_stem()));
    let mut sorted_subdirs: Vec<DirNode> = node.subdirs.drain(..).collect();
    sorted_subdirs.sort_by(|left, right| left.name.cmp(&right.name));
    node.subdirs = sorted_subdirs.into_iter().map(finalize).collect();
    node
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
    fn ancestor_paths_returns_each_directory_step() {
        let ancestors = ancestor_paths(&note("a/b/c/Deep.md"));
        assert_eq!(ancestors, vec!["a", "a/b", "a/b/c"]);
    }

    #[test]
    fn ancestor_paths_empty_for_root_notes() {
        assert!(ancestor_paths(&note("Index.md")).is_empty());
    }
}
