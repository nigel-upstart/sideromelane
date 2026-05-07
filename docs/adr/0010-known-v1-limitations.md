# ADR 0010: Known v1 Limitations

## Status

Accepted

## Context

The post-ship review identified several areas where a known constraint or
simplification was made in v1. This ADR records each one so the decisions are
explicit and searchable rather than implicit. None of these require immediate
action; each entry notes the condition under which a follow-up would be
warranted.

## Decision

Accept the following limitations for v1 and revisit them when the triggering
conditions are met.

### Force-directed graph layout is O(n²) and runs on the UI thread

`crates/sideromelane-app/src/graph_layout.rs` recomputes the layout on every
tick using a naive O(n²) force-integration loop. The implementation is
acceptable up to roughly 500 nodes; above approximately 2,000 nodes the UI
thread will stall noticeably. A future ADR will move graph layout into the
background indexer worker if folder sizes warrant the added concurrency
complexity.

### Wiki-link extractor is O(N²) on adversarial inputs

`crates/sideromelane-core/src/analysis.rs::extract_wiki_targets` exhibits
quadratic behavior when a note contains thousands of adjacent wiki-link
tokens, e.g. `[[a]][[b]]…` repeated at scale. Notes are user-owned content
and are not received from untrusted remote sources; the v1 threat model treats
this as a known limit rather than an exploitable vulnerability. If Sideromelane
ever gains import from arbitrary external sources, the extractor should be
replaced with a linear-pass parser.

### `validate_image_magic_bytes` does prefix-only matching

`crates/sideromelane-core/src/asset.rs::validate_image_magic_bytes` checks
only the first few bytes of a file header (e.g. any file starting with `BM`
is accepted as BMP). The function is defense-in-depth, not a full image
parser; the egui image decoder is the primary safety net and will reject
malformed content regardless of the magic-byte check. Upgrading to a
format-aware parser is deferred until there is evidence that the prefix check
is producing false positives or being actively bypassed.

### Concurrent instances race on the same note's temp file

The crash-safe write in `crates/sideromelane-app/src/io.rs` stages content
to a fixed sibling path (`*.md.tmp`). Two concurrent app instances writing
the same note would race on that temp path. Sideromelane is a single-instance
desktop app enforced by the macOS activation policy; this scenario cannot
arise in normal use. The limitation is recorded here in case a future
headless CLI or scripting interface bypasses the single-instance guard.

### Settings file is written with default umask permissions

`FolderSettings::save` creates `settings.json` with whatever umask the
process inherits from the shell or launchd. No secrets are stored in the
settings file today, so world-readable permissions are acceptable. If
per-folder settings ever hold sensitive material (API keys, auth tokens) the
save path should switch to `0o600` mode-restricted writes.

## Consequences

- These constraints are documented and will not surface as surprise findings
  in future reviews.
- Each entry has a concrete triggering condition that drives a follow-up ADR
  rather than open-ended technical debt.
- No code changes are made by this ADR; it is a record of accepted state.
