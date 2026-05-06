#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use sideromelane_core::{HybridSearchIndex, MarkdownNote, NoteId, SearchQuery};

/// Fixture path helper.
fn fixture(relative: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/folders")
        .join(relative)
}

#[test]
fn hybrid_search_includes_local_semantic_matches_without_exact_keywords() {
    let credit_id = NoteId::from_folder_relative_path("credit/Loan Review.md").unwrap();
    let garden_id = NoteId::from_folder_relative_path("garden/Planting.md").unwrap();

    let credit = MarkdownNote::parse(
        credit_id.clone(),
        "---\ntitle: Loan Review\ntags: [credit]\n---\n\nApprove applications after income review.\n",
    );
    let garden = MarkdownNote::parse(
        garden_id,
        "---\ntitle: Planting\ntags: [garden]\n---\n\nTomato seedlings need water.\n",
    );

    let index = HybridSearchIndex::from_notes(vec![credit, garden]);
    let results = index.search(&SearchQuery::text("approval"));

    assert_eq!(results[0].note_id(), &credit_id);
    assert_eq!(results[0].keyword_score(), 0);
    assert!(results[0].semantic_score() > 0.0);
    assert!(results[0].combined_score() > results[1].combined_score());
}

#[test]
fn large_note_parses_and_indexes_within_budget() {
    let path = fixture("large/large-note.md");
    let source = std::fs::read_to_string(&path).expect("large fixture must exist");
    assert!(
        source.len() >= 500 * 1024,
        "large fixture must be ≥500 KiB; got {} bytes",
        source.len()
    );

    let note_id = NoteId::from_folder_relative_path("large/large-note.md").unwrap();

    let start = std::time::Instant::now();
    let note = MarkdownNote::parse(note_id, source);
    let index = HybridSearchIndex::from_notes(vec![note]);
    let elapsed = start.elapsed();

    // Sanity: the index has a result for a term in the large note.
    let results = index.search(&SearchQuery::text("lorem"));
    assert!(!results.is_empty(), "large note must be searchable");

    // Soft latency budget: 2 s on debug builds.
    assert!(
        elapsed.as_secs() < 2,
        "large note parse+index took {elapsed:?} which exceeds the 2 s debug budget"
    );
}
