# ADR 0013: File Watch and Auto-Save

## Status

Accepted

## Context

Spec 0002 AC-4 calls for two features that share the same plumbing:

1. **Auto-save**: dirty notes should persist after a few seconds of typing
   inactivity so the manual Save button becomes optional.
2. **External-change detection**: when a file the app currently has open is
   modified outside Sideromelane (a `vim` save, a `git pull`, a sync agent),
   the user must either silently see the latest content (clean buffer) or
   be offered a non-blocking choice to reload or keep their unsaved edits.

The two are entangled because auto-save itself writes through the same
filesystem we are watching, so a naive implementation triggers spurious
"changed on disk" prompts every five seconds.

## Decision

### Auto-save

Each `NoteRecord` carries a `last_edit_at: Instant` that is stamped by
the `TextEdit` `changed` callback in the central pane (and by the image-drop
helper that mutates the buffer programmatically). Once per frame the
`auto_save_tick` walks dirty notes and calls the existing `safe_write`
primitive on every record whose `last_edit_at` is older than
`AppState::auto_save_debounce_secs` (default 5, clamped 1..=60). Successful
writes clear `dirty` and dispatch an `IndexerCommand::NoteChanged` so search,
backlinks, and graph indexes catch up. Errors surface in the status bar
without clearing `dirty`, so the next idle window retries.

The sweep is extracted into a free helper `auto_save_dirty_notes` that
takes `now: Instant` as a parameter so unit tests can synthesize
six-second-stale notes without `std::thread::sleep`.

### File watching

A new `crates/sideromelane-app/src/watcher.rs` wraps `notify-debouncer-mini`
0.7 with a small UI-friendly facade:

- `Watcher::new(root)` attaches a recursive watch with a 200 ms debounce
  window and starts a worker thread inside the debouncer.
- `Watcher::poll()` is non-blocking and mirrors `Indexer::poll` so the
  eframe update loop drains both channels each frame.
- `WatchEvent { path, kind: WatchKind::Modify | Other }` collapses notify's
  Create/Modify/Remove triplet into a single `Modify` category that the UI
  reacts to uniformly.

A watcher is opened on every `open_folder` and replaces the previous folder's
watcher (which is dropped, joining its worker thread cleanly). Watcher
construction failures (permission denied, kernel-watch limits) surface in
the status bar; the app remains usable with auto-save only.

### Conflict resolution

When a `WatchEvent` arrives:

1. If the timestamp is within 200 ms of our last self-write to that path,
   the event is dropped (suppresses our own auto-save loop).
2. The path is matched back to a `NoteRecord` (direct equality first; then
   a `file_name` fallback for FS-event canonicalization, e.g. macOS
   `/private/tmp/...`).
3. If the in-memory buffer is **clean**, the source is silently reloaded
   from disk and `last_edit_at` is reset.
4. If the buffer is **dirty**, the `NoteId` is pushed onto `pending_conflicts`
   (deduplicated) and an `egui::Window` is rendered for it next frame.

Each conflict window is non-blocking, non-modal, non-resizable, and
non-collapsible. It carries two buttons:

- **Reload from disk** — replaces `note.source` from disk, clears `dirty`,
  and drops the pending entry.
- **Keep mine** — drops the pending entry without changing the buffer; the
  next auto-save sweep will overwrite the disk version.

Closing the window with the X is treated as **Keep mine** because the
buffer already reflects the user's edits.

## Consequences

- Users no longer have to remember to save. The 5 s default debounce keeps
  typing snappy but bounds data loss after a crash to a single inactive
  window.
- The conflict modal is per-note, so the user can still navigate, edit
  other notes, and search while a conflict is outstanding.
- The 200 ms self-write suppression window is short enough that genuine
  external changes that race with our save still surface (the user types,
  saves, and `git pull` collides with that save in the same 200 ms — rare
  in practice and handled correctly the next time the watcher fires).
- `notify` and `notify-debouncer-mini` add a worker thread per open folder.
  Both shut down on Drop, so folder switches do not leak threads.
- The watcher is best-effort. On a platform where notify cannot attach
  (network filesystem, sandbox restrictions) auto-save still works, and
  external changes simply remain undetected until the user reopens the
  folder. This is consistent with the spec's "non-blocking" requirement.

## Alternatives considered

- **Polling at 1 Hz on a worker thread** — simpler dependency footprint
  but adds steady CPU and wakes the disk on every tick. Rejected on
  battery-life grounds.
- **`notify` directly without the mini debouncer** — would require us to
  re-implement the rename-over-temp coalescing that `safe_write` produces.
  The mini debouncer's 200 ms window collapses our own writes into a single
  event already, which dovetails with the self-write suppression window.
- **Modal blocking dialog on conflict** — fastest to ship but violates the
  spec's "must not block the entire app" requirement and forces context
  switches when the user has many notes open.

## Future work

- A diff view inside the conflict window so the user can see what changed
  on disk before committing to Reload or Keep. Out of scope for v1.
- Three-way merge of conflicting edits.
- Honoring `.sideromelaneignore` inside the watcher so events for ignored
  directories (e.g. `assets/`) are filtered before they reach the app.
