# Spec 0002: Shell Polish, Auto-Save, and Native Menu

## Status

Proposed — pending user confirmation before implementation.

## Objective

Eight related changes raise the app from "works" to "feels like a real tool":

1. **Outline → editor jump** on click, like HTML in-page anchors.
2. **Graph view** moves to a top-bar toggle, scoped to the current note's neighborhood (not the whole folder), and renders with a proper graph library.
3. **Native macOS menu bar** with File menu, recent-folder LRU, Open Folder, New File.
4. **Auto-save** dirty buffers after 5 s of inactivity; the manual Save button becomes optional. External-change detection drives a conflict modal when a file the user is editing is modified on disk.
5. **Better Markdown live preview** via a CommonMark/GFM rendering library.
6. **Word-wrap toggle that actually works** synchronously and immediately on toggle (UI-thread blocking is acceptable).
7. **Resizable internal panes**: drag the divider between Files and Search; remember the ratio.
8. **Startup behavior setting**: by default reload the last opened folder + last open note. Fallback default folder is `~/Documents/Sideromelane`. App-local persistence.

## Acceptance Criteria

### AC-1: Outline click jumps the editor
- Each outline row becomes a clickable affordance (RichText label rendered as a button with no border).
- On click, in **Raw** mode: set the editor cursor to the heading's byte offset, then `scroll_to_me` that line.
- On click, in **Live Preview** mode: set `active_block_index` to the block containing that heading and call `scroll_to_me` on its rect.
- Headings without a unique byte-offset target (degenerate empty result of `display_heading_text`) are not clickable.
- Tested via a unit helper: given a note source and a heading, return the byte offset and the live-preview block index.

### AC-2: Graph view is on demand and per-note
- Top toolbar gains a **Graph** toggle button next to Raw/Live Preview, mutually exclusive with them. When Graph is on, the central pane shows the graph; when Raw or Live Preview is on, it shows the editor. The right panel no longer hosts the graph.
- The graph shows only the **neighborhood of the currently selected note**: the note itself, every note linked from it (forward links), every note linking to it (backlinks), and one further hop on each side (so co-cited and co-citing notes appear). Configurable via a small "depth" slider in the graph view, default `1`.
- Use the `egui_graphs` crate (built on `petgraph`, force-directed, supports zoom/pan/clustering out of the box). We replace our hand-rolled `graph_layout` and `graph_view` modules.
- Selected node is highlighted; clicking another node selects that note in the editor, leaving the graph open with that node now central.
- "Empty graph" (note has no links and no backlinks) shows a small helper label, not a blank canvas.

### AC-3: Native macOS menu bar
- **File** menu:
  - "Open Folder…" (⌘O) — same action as the toolbar button.
  - "Recent Folders" submenu — LRU list of up to 10 recently opened folder paths. Click to open.
  - "New Note" (⌘N) — same as the New button.
  - "Save" (⌘S) — same as the Save button (still useful; auto-save is the default).
  - "Close" (⌘W) — closes the active note tab (no-op if not yet implemented).
- **Edit** menu: defer; egui handles cut/copy/paste in TextEdit.
- **View** menu:
  - "Show Graph" (⌘G) — toggles graph view.
  - "Word Wrap" (⌘⇧W) — toggles per-folder `editor_word_wrap`.
- Wired via the `muda` crate (cross-platform native menu, macOS-first feature parity).
- The toolbar buttons stay (the menu duplicates them; both surfaces work).

### AC-4: Auto-save and conflict resolution
- Each `NoteRecord` tracks `last_edit_at: Instant` updated on every change.
- A frame-tick check (cheap, runs in `update`) saves any note where `dirty && (now - last_edit_at) > AUTO_SAVE_DEBOUNCE` (5 s).
- The Save button stays for explicit saves but is no longer required for durability.
- A `notify`-backed file watcher (debounced via `notify-debouncer-mini`) is started on folder open. On `Modify` events for a note path:
  - If the in-memory buffer is **not dirty**: reload silently and update the editor.
  - If it is **dirty**: surface a modal `"<note> changed on disk. Reload from disk (lose your unsaved edits) / Keep your version (overwrite on next save)"`. Modal is non-blocking — user can interact with other notes; the conflict applies only when they next focus the affected note.
  - If the user has not interacted with the affected note in the last 30 s and the buffer is clean, treat as silent reload regardless of strict dirtiness (covers stale `last_edit_at`).
- **Race**: Auto-save fires while the watcher reports our own write — debounce 100 ms after `safe_write` returns to suppress the self-trigger; the watcher's debounce already absorbs most of it.
- Auto-save failures (e.g. permission errors) surface in the status bar, are retried at the next idle window, and do not clear `dirty`.

### AC-5: Better live-preview rendering
- Replace the hand-rolled block renderer in `main.rs` with `egui_commonmark` (uses `pulldown-cmark` internally). It handles ATX headings, lists, task lists, tables, fenced code, inline emphasis, links, and images.
- Wiki links `[[Note]]` and image embeds `![[image.png]]` are not CommonMark; render them as a pre-pass that substitutes `[[Note]]` with a CommonMark inline link `[Note](sideromelane://note/Note)` (and resolves on click) and `![[image.png]]` with `![image](file:///<absolute>/assets/image.png)`. The substitution is display-only; the source note is unchanged.
- Live-preview block-editor behavior (active block stays as Markdown source; inactive blocks render) remains. The renderer just gets prettier per-block.
- ADR 0003's promise (`pulldown-cmark` becomes the preferred parser) is now realized; update ADR 0003 status to "implemented in v1 via egui_commonmark".

### AC-6: Word-wrap toggle works synchronously
- Investigate the current `raw_editor` and `live_preview_editor` paths. The bug is that when `word_wrap == false`, the editor still wraps. Likely cause: the inner `TextEdit::desired_width(f32::INFINITY)` inside `ScrollArea::horizontal` is overridden by the parent `egui::Panel`'s inner-width clip.
- Fix: when `word_wrap == false`, set the `TextEdit` `layouter` explicitly to a non-wrapping layout, *or* use `egui::ScrollArea::both()` with explicit `auto_shrink([false, false])` and a `Layout::left_to_right` parent. Verify by typing a 200-char line and confirming a horizontal scroll bar appears with no wrapping.
- The toggle takes effect on the very next frame (no app restart) — already specified in 0001 but not actually working.
- Add an integration-style test that constructs a `TextEdit` with the wrap-off path and asserts the layout's resulting line count for a known long input is 1, not many. (This may not be feasible without a headless egui harness; if so, document as manually verified.)

### AC-7: Resizable internal panes (vertical split inside left panel)
- The left panel's `Files` (above) and `Search` (below) sections become resizable: a drag handle between them sets a vertical-split ratio.
- Persist the ratio in app-local state (not per-folder — it's a UI preference).
- Constrain to a min height of 80 px on each side; total = panel inner height.

### AC-8: Startup behavior + app-local state
- New app-local state at `<dirs::data_local_dir()>/sideromelane/state.json`:
  ```json
  {
    "version": 1,
    "startup_mode": "reload_last" | "new_note",
    "default_folder": "/Users/.../Documents/Sideromelane",
    "last_folder": "/Users/.../...",
    "last_note": "Cloud/Plan.md",
    "recent_folders": ["..."],
    "left_pane_split_ratio": 0.55
  }
  ```
- On launch:
  - If `startup_mode == "reload_last"` AND `last_folder` exists: open it; if `last_note` exists in it, select that note.
  - Else: open `default_folder` (creating it if missing) and start a new untitled note.
- A new **Preferences** window (no keyboard shortcut; reachable via the application menu) exposes:
  - Startup mode radio.
  - Default folder picker (`rfd`).
  - Auto-save debounce (s).
  - Word wrap default for new folders.
  - Word wrap toggle for the current folder (synced with the View menu and the right panel).
- The app writes app-local state on every meaningful change (folder open, note selection, splitter drag end, preferences save). Same atomic-write story as `FolderSettings::save`.

## Design Decisions (with overridable defaults)

- **Graph library**: `egui_graphs` ≥ 0.20. Pulls `petgraph` and a small set of egui-native helpers. *Override:* hand-rolled with `petgraph` if you'd rather not take an egui-graph dep.
- **Markdown renderer**: `egui_commonmark` ≥ 0.18 + `pulldown-cmark`. *Override:* pure `pulldown-cmark` + a thin egui-side renderer (more code, fewer deps).
- **File watcher**: `notify-debouncer-mini` ≥ 0.4. *Override:* polling at 1 Hz on a worker thread (much simpler, slightly worse UX).
- **macOS menu**: `muda` ≥ 0.13. *Override:* egui's in-window `MenuBar` (works today, less native-feeling, no Cmd-key shortcuts that survive focus changes).
- **App-local state location**: `dirs::data_local_dir()/sideromelane/state.json` (macOS resolves to `~/Library/Application Support/sideromelane/`). Adds `dirs` ≥ 5. *Override:* roll our own path resolution against `~/Library/Application Support/`.
- **Default folder**: `~/Documents/Sideromelane`. *Override:* `~/Documents/Notes` or any other path.
- **LRU length**: 10. *Override:* 5, 20, etc.
- **Auto-save debounce**: 5 s. *Override:* configurable via Preferences (per AC-8) — default 5.
- **Conflict-modal "kept" semantics**: marking "Keep your version" leaves the buffer dirty and continues; the next auto-save overwrites the on-disk version. *Override:* show a side-by-side diff first.
- **Outline click in Live Preview**: scrolls to and activates the heading's block. *Override:* leave the block inactive (heading still rendered), just scroll.

## New Dependency Asks (per SPEC.md "Ask first")

Confirm before I proceed:

1. `egui_graphs` (graph view)
2. `egui_commonmark` + `pulldown-cmark` is already authorized via ADR 0003 promise; confirm `egui_commonmark` specifically.
3. `notify` and `notify-debouncer-mini` (file watch)
4. `muda` (native macOS menu)
5. `dirs` (cross-platform data dir resolution)

All five are widely used, MIT/Apache-licensed, in `cargo-deny`'s permissive allow-list.

## Implementation Notes

Files most affected:

- `crates/sideromelane-app/src/main.rs` — toolbar reshuffle, mode enum gains `Graph`, auto-save tick, conflict-modal state, menu wiring, splitter, startup logic.
- `crates/sideromelane-app/src/{graph_layout,graph_view}.rs` — replaced by a single `graph_view.rs` that uses `egui_graphs` and an upstream-derived petgraph subset for the neighborhood.
- `crates/sideromelane-app/src/preview.rs` — new — wraps `egui_commonmark` per-block with the wiki-link/image-embed pre-pass.
- `crates/sideromelane-app/src/menu.rs` — new — `muda` setup + Recent-folders state.
- `crates/sideromelane-app/src/watcher.rs` — new — `notify-debouncer-mini` driver, sends `WatchEvent` to the indexer's UI-side event queue (or a separate one — see ADR 0011 below).
- `crates/sideromelane-app/src/state.rs` — new — app-local `AppState` load/save with atomic writes (re-uses the `safe_write` primitive — promote it from `app::io` to a small shared helper).
- `crates/sideromelane-app/src/preferences.rs` — new — Preferences window + persisted fields.
- `crates/sideromelane-app/src/outline.rs` — extended — heading `byte_offset_in_source(&str, level, text)` helper for the click-jump feature.
- `crates/sideromelane-core/src/folder_settings.rs` — *unchanged*. The new prefs are app-local, not per-folder.

New ADRs:

- `docs/adr/0011-native-macos-menu.md` — muda choice + the View/File menu surface.
- `docs/adr/0012-markdown-rendering.md` — egui_commonmark adoption, wiki/image-embed pre-pass.
- `docs/adr/0013-file-watch-and-auto-save.md` — notify-debouncer-mini + auto-save debounce + conflict-modal flow.
- `docs/adr/0014-app-local-state.md` — schema, location, and atomic-write story for `~/.../state.json`.

## Implementation Order (suggested)

To keep changes reviewable and to land lower-risk surfaces first:

1. **AC-6 word-wrap fix** — small, isolated, directly visible.
2. **AC-1 outline click-jump** — small, leans on AC-6 working.
3. **AC-7 resizable internal pane** — small, app-local-state shape introduced.
4. **AC-8 startup + preferences** — sets up the AppState plumbing the rest leans on.
5. **AC-3 native menu** — depends on AppState for recent-folders LRU.
6. **AC-4 auto-save + file watch** — depends on AppState (debounce config) and the safe-write primitive.
7. **AC-5 markdown rendering** — replaces a sizable chunk of `live_preview_editor`.
8. **AC-2 graph relocation + new lib** — replaces `graph_layout`/`graph_view` and reshuffles the central pane mode enum.

Each lands as its own commit (or commit pair: feature + tests). Existing test suite should remain green at every step.

## Testing Strategy

- **AC-1**: unit test `outline::byte_offset_for_heading(source, level, text) -> Option<usize>` against a fixture note with multiple headings.
- **AC-4**: integration test using `tempfile`. Spawn a watcher on a tempdir, write a file externally, assert the watcher event arrives within 500 ms. Separate test: `auto_save_fires_after_inactivity` — write source, advance time (use a `Clock` trait so tests can fast-forward), assert `safe_write` called.
- **AC-5**: snapshot test on `preview::transform_wiki_links(source) -> String` — pre-pass output is deterministic.
- **AC-6**: manual verification at minimum (egui rendering hard to assert headlessly). Document the manual steps in the PR description.
- **AC-7**: persisted `left_pane_split_ratio` round-trip via `state.rs` test.
- **AC-8**: load-defaults, parse-existing-state, schema-rejection (future version), atomic-save round-trip — same shape as `FolderSettings` tests.
- **AC-3**: smoke test — opening menu sends the same event as the toolbar button.
- **AC-2**: pure layout snapshot is harder with `egui_graphs`; assert the *neighborhood selection* helper (a graph-traversal function on `FolderIndex`) returns the expected node set for a fixture index.

## Boundaries

**Always:**
- `just check` and `just audit` clean before each commit.
- Hooks installed (`just install-hooks`).
- No `--no-verify`.
- New deps land via the workspace `Cargo.toml` and pass `cargo-deny`.

**Ask first:**
- Each of the five new deps (already listed above).
- Anything that requires a new platform crate beyond `muda`.

**Never:**
- Auto-save while the user is mid-keystroke (debounce inactive period only).
- Show a conflict modal that blocks the entire app — must be per-note, dismissible.
- Persist app-local state under the folder root (it's app-scope, not folder-scope).
- Rewrite Markdown source as a side-effect of rendering or auto-save.

## Out of Scope

- Diff view in the conflict modal (just reload-or-keep for now).
- Three-way merge of conflicting edits.
- Tabs for multiple open notes (still on the SPEC.md backlog, not blocked by this work).
- Mobile or non-macOS menu surfaces (muda's cross-platform path is fine, but we don't optimize for it in v1).
- Markdown extensions beyond CommonMark + GFM tables/task-lists (footnotes, math, etc.).
- Custom syntax themes for the editor / preview.

## Verification

When everything is implemented:

1. `cargo run -p sideromelane-app --release` and exercise:
   - Click an outline row → editor scrolls and cursor lands on the heading.
   - Toggle Graph from the toolbar → central pane shows graph for the current note's neighborhood. Click another node → it becomes the new center.
   - macOS menu bar: ⌘O opens picker; ⌘N creates a note; recent folders submenu populates after a few opens.
   - Edit a note, wait 5 s without typing → file is saved; status bar reflects it.
   - Modify the same file in `vim` from the terminal → conflict modal appears with reload/keep choices.
   - Toggle Word Wrap off in View menu → long line scrolls horizontally immediately.
   - Drag the splitter between Files and Search → ratio sticks across restart.
   - Quit and relaunch → last folder + last note re-opened.
2. `just check` and `just audit` clean.
3. All new ADRs committed alongside their implementation steps.
