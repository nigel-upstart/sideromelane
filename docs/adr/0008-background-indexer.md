# ADR 0008: Background Indexer

## Status

Accepted

## Context

The spec requires a typing-ready startup target under 75 ms and a typing-latency
target under 16 ms. Note discovery, Markdown parsing, hybrid search index
construction, and folder/graph index construction all scale with folder size
and cannot run synchronously on the UI thread without blowing those budgets.
Indexing also must not block initial editing on folder open.

## Decision

Move all indexing work onto a dedicated background worker thread owned by the
app crate.

- The worker is spawned with `std::thread::spawn` and lives in
  `crates/sideromelane-app/src/indexer.rs`.
- The UI sends `IndexerCommand` messages (`Rescan`, `NoteChanged`,
  `Shutdown`) over an `std::sync::mpsc::channel`.
- The worker publishes `IndexerEvent` messages back over a second channel
  and calls `egui::Context::request_repaint()` after each event so the UI
  wakes without polling.
- The egui `Context` is cloned into the worker at construction time.
- Two events make hydration progressive: `NotesDiscovered` lands the raw
  note set so the file panel populates quickly, and `IndexUpdated` follows
  with the freshly built `HybridSearchIndex` and `FolderIndex` which the
  UI swaps in atomically.
- The UI drains up to `MAX_INDEXER_EVENTS_PER_FRAME` events per frame to
  apply backpressure if a burst of background work arrives while the user
  is typing.

The implementation uses only the standard library. No `tokio`, no
`crossbeam`, and no file-watch crate (`notify`) for v1. At the expected
note counts (under 100k) std-mpsc and a single worker are sufficient.

### Eventual consistency

Search, backlinks, and graph reads always run against whatever indexes the
app currently holds. On folder open they start empty. Search input does
not trigger an index rebuild; results refresh when the next `IndexUpdated`
event arrives. Save dispatches `NoteChanged` so the user's edit is
reflected after the next round-trip through the worker.

### Failure model

If the worker panics or its channels disconnect, the UI keeps functioning
against its last-known indexes. The next folder open spawns a fresh
`Indexer`, which re-runs `Rescan` and rebuilds indexes from disk.
Indexer errors cannot corrupt notes: only saves go through `safe_write`
(see ADR 0009), and the indexer never writes back to disk.

## Consequences

- The startup and typing paths never wait on indexing.
- Index updates are eventually consistent. The UI must tolerate stale
  search/backlink/graph data and surface a lightweight "Indexing…"
  hint on first open.
- A future ADR can add filesystem watching (e.g. `notify`) so external
  edits trigger automatic `Rescan` without user action. v1 ships with
  manual rescan via `Open Folder` and the implicit rescans on new-note
  and image-insert flows.
- A future ADR can move the worker into a dedicated crate or core service
  if the indexer surface grows beyond the current handful of commands.
