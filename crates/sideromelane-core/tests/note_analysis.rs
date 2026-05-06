#![allow(missing_docs, clippy::unwrap_used)]

use sideromelane_core::{ImageEmbed, MarkdownNote, NoteAnalysis, NoteId, WikiLink};

#[test]
fn extracts_headings_links_and_image_embeds_from_note_body() {
    let note_id = NoteId::from_folder_relative_path("plans/Launch Plan.md").unwrap();
    let note = MarkdownNote::parse(
        note_id,
        r"---
title: Launch Plan
---

# Launch Plan

Ship the [[Release Checklist]] and review [[Roadmap]].

![[diagram.png]]

## Milestones
",
    );

    let analysis = NoteAnalysis::from_note(&note);

    assert_eq!(
        analysis
            .headings()
            .iter()
            .map(|heading| (heading.level(), heading.text()))
            .collect::<Vec<_>>(),
        vec![(1, "Launch Plan"), (2, "Milestones")]
    );
    assert_eq!(
        analysis
            .wiki_links()
            .iter()
            .map(WikiLink::target)
            .collect::<Vec<_>>(),
        vec!["Release Checklist", "Roadmap"]
    );
    assert_eq!(
        analysis
            .image_embeds()
            .iter()
            .map(ImageEmbed::target)
            .collect::<Vec<_>>(),
        vec!["diagram.png"]
    );
}
