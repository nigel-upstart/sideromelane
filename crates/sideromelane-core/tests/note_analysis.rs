#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use sideromelane_core::{
    Heading, ImageEmbed, MarkdownNote, NoteAnalysis, NoteId, Tag, WikiLink, merged_tags,
};

/// Fixture path helper.
fn fixture(relative: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/folders")
        .join(relative)
}

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

#[test]
fn fenced_code_block_content_is_not_extracted() {
    let path = fixture("valid/Project Overview.md");
    let source = std::fs::read_to_string(&path).expect("fixture must exist");
    let note_id = NoteId::from_folder_relative_path("valid/Project Overview.md").unwrap();
    let note = MarkdownNote::parse(note_id, source);
    let analysis = NoteAnalysis::from_note(&note);

    // Wiki links inside fenced blocks must NOT appear.
    let link_targets: Vec<&str> = analysis.wiki_links().iter().map(WikiLink::target).collect();
    assert!(
        !link_targets.contains(&"fake_link"),
        "fake_link inside backtick fence must not be extracted; got: {link_targets:?}"
    );
    assert!(
        !link_targets.contains(&"another_fake"),
        "another_fake inside backtick fence must not be extracted"
    );
    assert!(
        !link_targets.contains(&"also_fake"),
        "also_fake inside tilde fence must not be extracted"
    );

    // Headings inside fenced blocks must NOT appear.
    let heading_texts: Vec<&str> = analysis.headings().iter().map(Heading::text).collect();
    assert!(
        !heading_texts.contains(&"fake heading"),
        "fake heading inside backtick fence must not be extracted; got: {heading_texts:?}"
    );
    assert!(
        !heading_texts.contains(&"fake tilde heading"),
        "fake tilde heading inside tilde fence must not be extracted"
    );

    // Real links outside fences MUST still be present.
    assert!(
        link_targets.contains(&"Roadmap"),
        "real link [[Roadmap]] must be extracted"
    );
    assert!(
        link_targets.contains(&"Appendix"),
        "real link [[Appendix]] after fence must be extracted"
    );
}

#[test]
fn double_bang_is_not_an_image_embed() {
    let note_id = NoteId::from_folder_relative_path("scratch/test.md").unwrap();
    let note = MarkdownNote::parse(
        note_id,
        "Not an image: !![[fake.png]] but this is: ![[real.png]]\n",
    );
    let analysis = NoteAnalysis::from_note(&note);

    let embed_targets: Vec<&str> = analysis
        .image_embeds()
        .iter()
        .map(ImageEmbed::target)
        .collect();
    assert!(
        !embed_targets.contains(&"fake.png"),
        "!![[fake.png]] must not be treated as image embed"
    );
    assert!(
        embed_targets.contains(&"real.png"),
        "![[real.png]] must be treated as image embed"
    );
}

#[test]
fn wiki_link_alias_and_anchor_are_parsed() {
    let note_id = NoteId::from_folder_relative_path("scratch/test.md").unwrap();
    let note = MarkdownNote::parse(
        note_id,
        "[[Note|display alias]] [[Other#section]] [[Both#anchor|label]] [[Bare]]\n",
    );
    let analysis = NoteAnalysis::from_note(&note);
    let links = analysis.wiki_links();

    assert_eq!(links[0].target(), "Note");
    assert_eq!(links[0].alias(), Some("display alias"));
    assert_eq!(links[0].anchor(), None);

    assert_eq!(links[1].target(), "Other");
    assert_eq!(links[1].anchor(), Some("section"));
    assert_eq!(links[1].alias(), None);

    assert_eq!(links[2].target(), "Both");
    assert_eq!(links[2].anchor(), Some("anchor"));
    assert_eq!(links[2].alias(), Some("label"));

    assert_eq!(links[3].target(), "Bare");
    assert_eq!(links[3].anchor(), None);
    assert_eq!(links[3].alias(), None);
}

#[test]
fn merged_tags_returns_sorted_deduplicated_union_of_frontmatter_and_inline() {
    let note_id = NoteId::from_folder_relative_path("notes/K8s.md").unwrap();
    let note = MarkdownNote::parse(
        note_id,
        "---\ntags: [datadog, kubernetes]\n---\n\nSee #kubernetes and #sumologic.\n",
    );
    let analysis = NoteAnalysis::from_note(&note);
    let tags = merged_tags(&note, &analysis);
    let names: Vec<&str> = tags.iter().map(Tag::name).collect();
    // frontmatter: [datadog, kubernetes]; inline: [kubernetes, sumologic]
    // union, sorted, deduped → [datadog, kubernetes, sumologic]
    assert_eq!(names, ["datadog", "kubernetes", "sumologic"]);
}

#[test]
fn merged_tags_with_only_inline_tags_returns_those_tags() {
    let note_id = NoteId::from_folder_relative_path("notes/Inline.md").unwrap();
    let note = MarkdownNote::parse(
        note_id,
        "---\ntitle: Inline Only\n---\n\nUses #celery and #datadog.\n",
    );
    let analysis = NoteAnalysis::from_note(&note);
    let tags = merged_tags(&note, &analysis);
    let names: Vec<&str> = tags.iter().map(Tag::name).collect();
    assert_eq!(names, ["celery", "datadog"]);
}
