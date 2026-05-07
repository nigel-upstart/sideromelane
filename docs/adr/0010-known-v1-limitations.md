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

### Force-directed graph layout is O(n²) and runs on the UI thread (resolved)

Resolved in Slice 8 of spec 0002. The hand-rolled `graph_layout.rs` has been
removed; the graph view is now driven by the published `egui_graphs` crate
and is scoped to the 1-hop neighborhood of the focused note rather than the
full folder, so the previous O(n²) full-folder layout cost is no longer on
the UI's hot path. Any remaining layout cost is internal to `egui_graphs`
and is its concern, not Sideromelane's. A separate background-layout
follow-up is no longer warranted.

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

## Deferred review findings (post-Spec 0002)

The following items surfaced during the Wave-1 review pass after Spec 0002
landed. None are user-visible today; each is recorded with a triggering
condition under which a follow-up should land.

### Graph `NeighborhoodSignature` clones+sorts every frame

`crates/sideromelane-app/src/graph_view.rs:62-65` rebuilds the neighborhood
signature on every frame the graph view renders, which means a clone and
sort of the focused note's neighbor list at frame rate. Caching by
`(focus, folder_index_revision)` is the right fix and keeps the data
correct across folder reloads. Pass on now since the graph mode is not on
a hot UI path; revisit when the graph is heavily used in practice or when
folders with very large neighborhoods become common.

### `app_state.last_note` allocates per frame in selection comparison

`crates/sideromelane-app/src/main.rs:225-233` recomputes the selected
note's relative path via `.display().to_string()` every frame to compare
against `app_state.last_note`. Cache the selection as `Option<NoteId>` to
avoid the per-frame allocation. Steady-state path with no observable
impact today; pick this up the next time `app_state` writes are touched.

### muda menu labels don't strip control characters

`crates/sideromelane-app/src/menu.rs::display_label` builds menu titles
from path components without stripping ASCII control characters. macOS
`NSMenuItem` titles strip control chars at the OS level so the rendered
label is benign, but a defense-in-depth fix is to strip on our side as
well. Pass for now; revisit if we add non-macOS targets where the OS
guarantee no longer applies.

## Consequences

- These constraints are documented and will not surface as surprise findings
  in future reviews.
- Each entry has a concrete triggering condition that drives a follow-up ADR
  rather than open-ended technical debt.
- No code changes are made by this ADR; it is a record of accepted state.
