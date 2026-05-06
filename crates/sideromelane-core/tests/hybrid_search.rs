#![allow(missing_docs, clippy::unwrap_used)]

use sideromelane_core::{HybridSearchIndex, MarkdownNote, NoteId, SearchQuery};

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
