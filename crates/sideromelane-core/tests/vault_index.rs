#![allow(missing_docs, clippy::unwrap_used)]

use sideromelane_core::{MarkdownNote, NoteId, VaultIndex};

#[test]
fn builds_backlinks_and_graph_edges_from_resolved_wiki_links() {
    let launch_id = NoteId::from_vault_relative_path("plans/Launch Plan.md").unwrap();
    let checklist_id = NoteId::from_vault_relative_path("plans/Release Checklist.md").unwrap();
    let roadmap_id = NoteId::from_vault_relative_path("plans/Roadmap.md").unwrap();

    let launch = MarkdownNote::parse(
        launch_id.clone(),
        "# Launch Plan\n\nUse the [[Release Checklist]] and [[Missing Note]].\n",
    );
    let checklist = MarkdownNote::parse(
        checklist_id.clone(),
        "# Release Checklist\n\nReview the [[Launch Plan]].\n",
    );
    let roadmap = MarkdownNote::parse(roadmap_id.clone(), "# Roadmap\n\nNo links yet.\n");

    let index = VaultIndex::from_notes(vec![launch, checklist, roadmap]);

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
