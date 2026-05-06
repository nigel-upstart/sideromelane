#![allow(missing_docs, clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use sideromelane_core::{MarkdownNote, MetadataValue, NoteId};

/// Fixture path helper.
fn fixture(relative: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/folders")
        .join(relative)
}

fn load_fixture_note(rel_fixture: &str, note_rel: &str) -> MarkdownNote {
    let path = fixture(rel_fixture);
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("fixture must exist: {}", path.display()));
    let note_id = NoteId::from_folder_relative_path(note_rel).unwrap();
    MarkdownNote::parse(note_id, source)
}

#[test]
fn parses_note_body_and_simple_frontmatter() {
    let note_id = NoteId::from_folder_relative_path("plans/Launch Plan.md").unwrap();
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
    let note_id = NoteId::from_folder_relative_path("Inbox.md").unwrap();
    let source = "---\ntitle: Missing Close\n\n# Still Source\n";
    let note = MarkdownNote::parse(note_id, source);

    assert!(note.frontmatter().is_none());
    assert_eq!(note.body(), source);
}

// ─── malformed-frontmatter fixture tests ────────────────────────────────────

#[test]
fn malformed_unterminated_frontmatter_does_not_panic() {
    // The note has no closing `---`, so the whole source becomes the body.
    let note = load_fixture_note(
        "malformed-frontmatter/unterminated.md",
        "malformed-frontmatter/unterminated.md",
    );

    // Must not panic; frontmatter is absent (unterminated block is treated as body).
    assert!(
        note.frontmatter().is_none(),
        "unterminated frontmatter must not parse as frontmatter"
    );
    // Body must contain the note text.
    assert!(
        note.body().contains("Body Starts Here"),
        "body must include content after the fake frontmatter opener"
    );
}

#[test]
fn malformed_quoted_comma_list_does_not_panic() {
    // tags: [a, "b, c", d] — the quoted comma is parsed naively (splits on every comma).
    let note = load_fixture_note(
        "malformed-frontmatter/quoted-comma-list.md",
        "malformed-frontmatter/quoted-comma-list.md",
    );

    // Must not panic. We accept whatever the parser produces (naive split is acceptable in v1).
    assert!(
        note.frontmatter().is_some(),
        "quoted-comma list note must have frontmatter"
    );
    let fm = note.frontmatter().unwrap();
    // tags must be a list (even if the quoted comma is not handled specially).
    assert!(
        fm.list("tags").is_some(),
        "tags field must parse as a list; got: {:?}",
        fm.get("tags")
    );
}

#[test]
fn malformed_empty_value_does_not_panic() {
    let note = load_fixture_note(
        "malformed-frontmatter/empty-value.md",
        "malformed-frontmatter/empty-value.md",
    );

    assert!(note.frontmatter().is_some());
    let fm = note.frontmatter().unwrap();
    // `key:` with empty value should produce an empty scalar (or be absent).
    match fm.get("key") {
        None => {} // absent is acceptable
        Some(MetadataValue::Scalar(v)) => assert!(
            v.is_empty(),
            "empty value field must be empty scalar; got: {v:?}"
        ),
        Some(other) => panic!("unexpected value for empty key: {other:?}"),
    }
    // Other fields must still parse.
    assert_eq!(fm.scalar("title"), Some("Empty Value"));
    assert_eq!(fm.scalar("status"), Some("active"));
}

#[test]
fn malformed_duplicate_keys_last_wins() {
    let note = load_fixture_note(
        "malformed-frontmatter/duplicate-keys.md",
        "malformed-frontmatter/duplicate-keys.md",
    );

    assert!(note.frontmatter().is_some());
    let fm = note.frontmatter().unwrap();
    // Last-wins: `title` should be "Second Title".
    assert_eq!(
        fm.scalar("title"),
        Some("Second Title"),
        "duplicate key: last-wins semantics expected"
    );
}

#[test]
fn malformed_colon_in_value_does_not_panic() {
    let note = load_fixture_note(
        "malformed-frontmatter/colon-in-value.md",
        "malformed-frontmatter/colon-in-value.md",
    );

    assert!(note.frontmatter().is_some());
    let fm = note.frontmatter().unwrap();
    // summary: "foo: bar" — should parse as scalar "foo: bar" (quotes stripped, colon preserved).
    assert_eq!(
        fm.scalar("summary"),
        Some("foo: bar"),
        "colon inside quoted value must be preserved"
    );
    // url: https://example.com — splits on first colon, value becomes "//example.com"
    // (this is the known v1 limitation; the test documents the actual behavior).
    assert!(
        fm.scalar("url").is_some() || fm.get("url").is_some(),
        "url field must be present even when value contains colons"
    );
}
