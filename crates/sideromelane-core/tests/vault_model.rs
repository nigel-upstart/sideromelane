#![allow(missing_docs, clippy::unwrap_used)]

use std::path::Path;

use sideromelane_core::NoteId;

#[test]
fn note_id_accepts_nested_markdown_paths() {
    let note_id = NoteId::from_vault_relative_path("notes/Launch Plan.md").unwrap();

    assert_eq!(note_id.relative_path(), Path::new("notes/Launch Plan.md"));
    assert_eq!(note_id.file_stem(), "Launch Plan");
}

#[test]
fn note_id_rejects_paths_that_are_not_safe_vault_notes() {
    for path in [
        "",
        ".",
        "/tmp/secret.md",
        "../secret.md",
        "notes/../../secret.md",
        "notes/",
        "assets/image.png",
    ] {
        assert!(
            NoteId::from_vault_relative_path(path).is_err(),
            "expected {path:?} to be rejected"
        );
    }
}
