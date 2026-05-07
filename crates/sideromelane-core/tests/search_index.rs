#![allow(missing_docs, clippy::unwrap_used)]

use sideromelane_core::{MarkdownNote, NoteId, SearchIndex, SearchQuery};

#[test]
fn keyword_search_ranks_text_matches_and_applies_filters() {
    let launch_id = NoteId::from_folder_relative_path("plans/Launch Plan.md").unwrap();
    let roadmap_id = NoteId::from_folder_relative_path("plans/Roadmap.md").unwrap();
    let retro_id = NoteId::from_folder_relative_path("meetings/Retro.md").unwrap();

    let launch = MarkdownNote::parse(
        launch_id.clone(),
        "---\ntitle: Launch Plan\ntags: [planning, product]\nstatus: draft\n---\n\n# Launch\n\nShip the checklist.\n",
    );
    let roadmap = MarkdownNote::parse(
        roadmap_id,
        "---\ntitle: Product Roadmap\ntags: [product]\nstatus: active\n---\n\nLaunch sequencing notes.\n",
    );
    let retro = MarkdownNote::parse(
        retro_id.clone(),
        "---\ntitle: Retro\ntags: [meetings]\nstatus: draft\n---\n\nLaunch review.\n",
    );

    let index = SearchIndex::from_notes(vec![launch, roadmap, retro]);

    let launch_results = index.search(&SearchQuery::text("launch"));
    assert_eq!(launch_results[0].note_id(), &launch_id);
    assert!(launch_results[0].score() > launch_results[1].score());

    let filtered_results = index.search(
        &SearchQuery::text("launch")
            .with_tag("planning")
            .with_field("status", "draft"),
    );
    assert_eq!(filtered_results.len(), 1);
    assert_eq!(filtered_results[0].note_id(), &launch_id);

    let file_results = index.search(&SearchQuery::empty().with_file_name("retro"));
    assert_eq!(file_results.len(), 1);
    assert_eq!(file_results[0].note_id(), &retro_id);
}

#[test]
fn inline_tag_filter_matches_note_without_frontmatter_tag() {
    let note_id = NoteId::from_folder_relative_path("notes/K8s.md").unwrap();
    let note = MarkdownNote::parse(
        note_id.clone(),
        "---\ntitle: K8s Notes\n---\n\nSee #kubernetes for context.\n",
    );

    let index = SearchIndex::from_notes(vec![note]);

    let results = index.search(&SearchQuery::empty().with_tag("kubernetes"));
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].note_id(), &note_id);
}
