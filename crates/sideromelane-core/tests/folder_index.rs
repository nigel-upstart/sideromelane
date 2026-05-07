#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use sideromelane_core::{FolderIndex, MarkdownNote, NoteId};

/// Fixture path helper.
fn fixture(relative: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/folders")
        .join(relative)
}

#[test]
fn builds_backlinks_and_graph_edges_from_resolved_wiki_links() {
    let launch_id = NoteId::from_folder_relative_path("plans/Launch Plan.md").unwrap();
    let checklist_id = NoteId::from_folder_relative_path("plans/Release Checklist.md").unwrap();
    let roadmap_id = NoteId::from_folder_relative_path("plans/Roadmap.md").unwrap();

    let launch = MarkdownNote::parse(
        launch_id.clone(),
        "# Launch Plan\n\nUse the [[Release Checklist]] and [[Missing Note]].\n",
    );
    let checklist = MarkdownNote::parse(
        checklist_id.clone(),
        "# Release Checklist\n\nReview the [[Launch Plan]].\n",
    );
    let roadmap = MarkdownNote::parse(roadmap_id.clone(), "# Roadmap\n\nNo links yet.\n");

    let index = FolderIndex::from_notes(vec![launch, checklist, roadmap]);

    assert_eq!(
        index
            .backlinks_to(&launch_id)
            .iter()
            .map(|backlink| backlink.source().file_stem())
            .collect::<Vec<_>>(),
        vec!["Release Checklist"]
    );
    assert_eq!(
        index
            .backlinks_to(&checklist_id)
            .iter()
            .map(|backlink| backlink.source().file_stem())
            .collect::<Vec<_>>(),
        vec!["Launch Plan"]
    );
    assert!(index.backlinks_to(&roadmap_id).is_empty());

    assert_eq!(
        index
            .graph()
            .nodes()
            .iter()
            .map(|node| node.note_id().file_stem())
            .collect::<Vec<_>>(),
        vec!["Launch Plan", "Release Checklist", "Roadmap"]
    );
    assert_eq!(
        index
            .graph()
            .edges()
            .iter()
            .map(|edge| (edge.source().file_stem(), edge.target().file_stem()))
            .collect::<Vec<_>>(),
        vec![
            ("Launch Plan", "Release Checklist"),
            ("Release Checklist", "Launch Plan"),
        ]
    );
}

#[test]
fn duplicate_stems_produce_non_empty_ambiguous_targets() {
    // Load the duplicate-stems fixture folder.
    let base = fixture("duplicate-stems");

    // Build NoteIds and notes for all three fixture files.
    let paths = [
        ("a/Roadmap.md", "a/Roadmap.md"),
        ("b/Roadmap.md", "b/Roadmap.md"),
        ("Index.md", "Index.md"),
    ];

    let notes: Vec<MarkdownNote> = paths
        .iter()
        .map(|(rel, _)| {
            let abs = base.join(rel);
            let source = std::fs::read_to_string(&abs)
                .unwrap_or_else(|_| panic!("fixture must exist: {}", abs.display()));
            let note_id = NoteId::from_folder_relative_path(rel).unwrap();
            MarkdownNote::parse(note_id, source)
        })
        .collect();

    let index = FolderIndex::from_notes(notes);

    let ambiguous = index.ambiguous_targets();
    assert!(
        !ambiguous.is_empty(),
        "duplicate-stems fixture must produce non-empty ambiguous_targets()"
    );
    assert!(
        ambiguous.contains_key("Roadmap"),
        "ambiguous_targets() must contain 'Roadmap'; got: {ambiguous:?}"
    );
    assert_eq!(
        ambiguous["Roadmap"].len(),
        2,
        "two notes share the 'Roadmap' stem"
    );
}

#[test]
fn neighborhood_one_hop_collects_direct_neighbors() {
    let a_id = NoteId::from_folder_relative_path("A.md").unwrap();
    let b_id = NoteId::from_folder_relative_path("B.md").unwrap();
    let c_id = NoteId::from_folder_relative_path("C.md").unwrap();

    let a = MarkdownNote::parse(a_id.clone(), "# A\n[[B]] [[C]]\n");
    let b = MarkdownNote::parse(b_id, "# B\n");
    let c = MarkdownNote::parse(c_id, "# C\n");

    let index = FolderIndex::from_notes(vec![a, b, c]);
    let neighborhood = index.neighborhood(&a_id, 1);

    let mut node_stems: Vec<&str> = neighborhood.nodes.iter().map(NoteId::file_stem).collect();
    node_stems.sort_unstable();
    assert_eq!(node_stems, vec!["A", "B", "C"]);
    assert_eq!(neighborhood.edges.len(), 2);
}

#[test]
fn neighborhood_depth_bounds_traversal() {
    let a_id = NoteId::from_folder_relative_path("A.md").unwrap();
    let b_id = NoteId::from_folder_relative_path("B.md").unwrap();
    let c_id = NoteId::from_folder_relative_path("C.md").unwrap();

    let a = MarkdownNote::parse(a_id.clone(), "# A\n[[B]]\n");
    let b = MarkdownNote::parse(b_id, "# B\n[[C]]\n");
    let c = MarkdownNote::parse(c_id, "# C\n");

    let index = FolderIndex::from_notes(vec![a, b, c]);

    let depth_one = index.neighborhood(&a_id, 1);
    let mut one_stems: Vec<&str> = depth_one.nodes.iter().map(NoteId::file_stem).collect();
    one_stems.sort_unstable();
    assert_eq!(one_stems, vec!["A", "B"]);

    let depth_two = index.neighborhood(&a_id, 2);
    let mut two_stems: Vec<&str> = depth_two.nodes.iter().map(NoteId::file_stem).collect();
    two_stems.sort_unstable();
    assert_eq!(two_stems, vec!["A", "B", "C"]);
}

#[test]
fn neighborhood_unknown_focus_returns_singleton() {
    let known_id = NoteId::from_folder_relative_path("Known.md").unwrap();
    let known = MarkdownNote::parse(known_id.clone(), "# Known\n");
    let index = FolderIndex::from_notes(vec![known]);

    let stranger_id = NoteId::from_folder_relative_path("Stranger.md").unwrap();
    let neighborhood = index.neighborhood(&stranger_id, 3);

    assert_eq!(neighborhood.nodes, vec![stranger_id]);
    assert!(neighborhood.edges.is_empty());

    let zero_depth = index.neighborhood(&known_id, 0);
    assert_eq!(zero_depth.nodes, vec![known_id]);
    assert!(zero_depth.edges.is_empty());
}
