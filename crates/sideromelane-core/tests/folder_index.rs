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
