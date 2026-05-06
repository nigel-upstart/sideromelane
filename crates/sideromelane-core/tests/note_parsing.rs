#![allow(missing_docs, clippy::unwrap_used)]

use sideromelane_core::{MarkdownNote, MetadataValue, NoteId};

#[test]
fn parses_note_body_and_simple_frontmatter() {
    let note_id = NoteId::from_vault_relative_path("plans/Launch Plan.md").unwrap();
    let note = MarkdownNote::parse(
        note_id,
        r#"---
title: Launch Plan
tags: [planning, product]
status: draft
priority: "high"
---

# Launch Plan

Ship the [[Release Checklist]].
"#,
    );

    let frontmatter = note.frontmatter().unwrap();

    assert_eq!(
        note.body(),
        "# Launch Plan\n\nShip the [[Release Checklist]].\n"
    );
    assert_eq!(frontmatter.scalar("title"), Some("Launch Plan"));
    assert_eq!(frontmatter.scalar("status"), Some("draft"));
    assert_eq!(frontmatter.scalar("priority"), Some("high"));
    assert_eq!(
        frontmatter.list("tags"),
        Some([String::from("planning"), String::from("product")].as_slice())
    );
    assert_eq!(
        frontmatter.get("priority"),
        Some(&MetadataValue::Scalar(String::from("high")))
    );
}

#[test]
fn treats_text_without_complete_frontmatter_as_body() {
    let note_id = NoteId::from_vault_relative_path("Inbox.md").unwrap();
    let source = "---\ntitle: Missing Close\n\n# Still Source\n";
    let note = MarkdownNote::parse(note_id, source);

    assert!(note.frontmatter().is_none());
    assert_eq!(note.body(), source);
}
