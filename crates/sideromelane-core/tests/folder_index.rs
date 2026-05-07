#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use sideromelane_core::{FolderIndex, GraphNode, MarkdownNote, NoteId, Tag};

/// Fixture path helper.
fn fixture(relative: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/folders")
        .join(relative)
}

fn note_node(id: &NoteId) -> GraphNode {
    GraphNode::Note {
        note_id: id.clone(),
    }
}

fn note_focus(id: &NoteId) -> GraphNode {
    note_node(id)
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

    // Only note nodes (no tags in these fixtures).
    let node_stems: Vec<&str> = index
        .graph()
        .nodes()
        .iter()
        .filter_map(|node| node.as_note())
        .map(NoteId::file_stem)
        .collect();
    assert_eq!(
        node_stems,
        vec!["Launch Plan", "Release Checklist", "Roadmap"]
    );

    // Only note→note edges in these fixtures.
    let edge_stems: Vec<(&str, &str)> = index
        .graph()
        .edges()
        .iter()
        .filter_map(|edge| {
            let src = edge.source().as_note()?.file_stem();
            let tgt = edge.target().as_note()?.file_stem();
            Some((src, tgt))
        })
        .collect();
    assert_eq!(
        edge_stems,
        vec![
            ("Launch Plan", "Release Checklist"),
            ("Release Checklist", "Launch Plan"),
        ]
    );
}

#[test]
fn duplicate_stems_produce_non_empty_ambiguous_targets() {
    let base = fixture("duplicate-stems");

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
    let neighborhood = index.neighborhood(&note_focus(&a_id), 1);

    // No inline tags in these notes, so only note nodes.
    let mut node_stems: Vec<&str> = neighborhood
        .nodes
        .iter()
        .filter_map(|node| node.as_note())
        .map(NoteId::file_stem)
        .collect();
    node_stems.sort_unstable();
    assert_eq!(node_stems, vec!["A", "B", "C"]);

    // Only note→note edges.
    assert_eq!(
        neighborhood
            .edges
            .iter()
            .filter(|(s, t)| s.as_note().is_some() && t.as_note().is_some())
            .count(),
        2
    );
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

    let depth_one = index.neighborhood(&note_focus(&a_id), 1);
    let mut one_stems: Vec<&str> = depth_one
        .nodes
        .iter()
        .filter_map(|node| node.as_note())
        .map(NoteId::file_stem)
        .collect();
    one_stems.sort_unstable();
    assert_eq!(one_stems, vec!["A", "B"]);

    let depth_two = index.neighborhood(&note_focus(&a_id), 2);
    let mut two_stems: Vec<&str> = depth_two
        .nodes
        .iter()
        .filter_map(|node| node.as_note())
        .map(NoteId::file_stem)
        .collect();
    two_stems.sort_unstable();
    assert_eq!(two_stems, vec!["A", "B", "C"]);
}

#[test]
fn neighborhood_unknown_focus_returns_singleton() {
    let known_id = NoteId::from_folder_relative_path("Known.md").unwrap();
    let known = MarkdownNote::parse(known_id.clone(), "# Known\n");
    let index = FolderIndex::from_notes(vec![known]);

    let stranger_id = NoteId::from_folder_relative_path("Stranger.md").unwrap();
    let neighborhood = index.neighborhood(&note_focus(&stranger_id), 3);

    assert_eq!(neighborhood.nodes, vec![note_node(&stranger_id)]);
    assert!(neighborhood.edges.is_empty());

    let zero_depth = index.neighborhood(&note_focus(&known_id), 0);
    assert_eq!(zero_depth.nodes, vec![note_node(&known_id)]);
    assert!(zero_depth.edges.is_empty());
}

#[test]
fn tag_index_maps_tags_to_notes() {
    let a_id = NoteId::from_folder_relative_path("A.md").unwrap();
    let b_id = NoteId::from_folder_relative_path("B.md").unwrap();
    let c_id = NoteId::from_folder_relative_path("C.md").unwrap();

    let a = MarkdownNote::parse(a_id.clone(), "---\ntags: [rust, systems]\n---\n");
    let b = MarkdownNote::parse(b_id.clone(), "Body text with #rust and #web inline.\n");
    let c = MarkdownNote::parse(c_id, "No tags here.\n");

    let index = FolderIndex::from_notes(vec![a, b, c]);
    let tag_index = index.tag_index();

    let rust_tag = Tag::new("rust").expect("valid");
    let systems_tag = Tag::new("systems").expect("valid");
    let web_tag = Tag::new("web").expect("valid");

    assert!(tag_index.contains_key(&rust_tag), "rust should be indexed");
    assert!(
        tag_index.contains_key(&systems_tag),
        "systems should be indexed"
    );
    assert!(tag_index.contains_key(&web_tag), "web should be indexed");

    let mut rust_notes = tag_index[&rust_tag].clone();
    rust_notes.sort();
    assert_eq!(rust_notes, vec![a_id.clone(), b_id]);

    assert_eq!(tag_index.get(&systems_tag).unwrap(), &vec![a_id]);
    assert!(!tag_index.contains_key(&Tag::new("nonexistent").unwrap()));
    assert!(
        !tag_index.contains_key(&Tag::new("c").unwrap()),
        "C has no tags"
    );
}

#[test]
fn neighborhood_tag_focus_returns_notes_using_tag() {
    let a_id = NoteId::from_folder_relative_path("A.md").unwrap();
    let b_id = NoteId::from_folder_relative_path("B.md").unwrap();
    let c_id = NoteId::from_folder_relative_path("C.md").unwrap();

    // A and B use #shared; C does not.
    let a = MarkdownNote::parse(a_id.clone(), "Text #shared here.\n");
    let b = MarkdownNote::parse(b_id.clone(), "Also #shared here.\n");
    let c = MarkdownNote::parse(c_id, "No tags.\n");

    let index = FolderIndex::from_notes(vec![a, b, c]);

    let tag = Tag::new("shared").expect("valid");
    let tag_focus = GraphNode::Tag { tag };
    let neighborhood = index.neighborhood(&tag_focus, 1);

    // Should contain the tag node + A + B (not C).
    assert!(neighborhood.nodes.contains(&tag_focus));
    assert!(neighborhood.nodes.contains(&note_node(&a_id)));
    assert!(neighborhood.nodes.contains(&note_node(&b_id)));
    assert_eq!(neighborhood.nodes.len(), 3);

    // Two note→tag edges.
    assert_eq!(neighborhood.edges.len(), 2);
}
