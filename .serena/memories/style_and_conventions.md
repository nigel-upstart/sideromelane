# Code Style and Conventions

- Language: Rust (edition 2021)
- No comments by default; only add when the WHY is non-obvious
- No multi-line doc comments; one-line `///` max on public items
- Validation at system boundaries: `Tag::new` validates input; internal code trusts invariants
- Error handling: `Result` for fallible domain ops; `expect` only when invariant is guaranteed
- Test files: `crates/sideromelane-core/tests/` for integration, inline `#[cfg(test)]` for unit
- Test convention: `#![allow(missing_docs, clippy::unwrap_used)]` in test files
- `merged_tags(note, analysis)` is the single source of truth for a note's full tag set
- Clippy: `-D warnings` (all warnings are errors)
- Formatting: `cargo fmt --all`
