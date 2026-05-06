# Spec 0001: UI Polish — Files Tree, Editor Wrap, Outline Sizing, Graph Layout

## Status

Proposed — pending user confirmation before implementation.

## Objective

The current shell, while functional, fails on four UX points that are visible the moment a real
folder is opened (see screenshots in the originating session):

1. **Files panel** is a flat list of relative paths (e.g. `Cloud/Q4 Plan Feedback.md`). It should
   render as a collapsible tree with folder icons and an indent per level.
2. **Editor pane** wraps at an apparent fixed width, ignoring the middle pane's resize. Long
   lines either need soft wrap to the pane width or a horizontal scroll bar — and which one is a
   user preference.
3. **Outline panel** shows raw Markdown source (`## **Heading**`). It should render headings as
   sized labels (H1 largest, H6 smallest), with the `#` and surrounding `**`/`*` emphasis markers
   stripped.
4. **Graph view** renders all nodes around the perimeter of a single circle, producing an
   unreadable ring. We want a force-directed layout closer to Obsidian's: dense hubs, organic
   clusters, sparse periphery, with zoom and pan.

Done means a user opening a real folder gets an interface that looks intentional, not a debug view.

## Acceptance Criteria

### AC-1: Files panel is a tree
- A folder with files at multiple depths renders as a tree where:
  - The folder root is implicit (not shown as a row).
  - Each subdirectory is a row with a folder icon (`▶`/`▼` chevron acceptable as the v1 icon)
    and the directory name (not the full path).
  - Clicking a folder row toggles expand/collapse.
  - Files indent under their parent folder by ~14–18 px per level.
  - Files are leaf rows with no chevron and a small file glyph (or no glyph if it costs an asset).
- Initial state: all folders collapsed *except* the folder containing the currently selected
  note, plus its ancestors.
- Persisted state (across app launches): the set of explicitly expanded folder paths, stored in
  per-folder settings under `ui.tree_expanded_paths: ["a", "a/b", …]`. Empty by default.
- Selection in the tree continues to update `folder.selected` exactly as the flat list does today.
- Search results (when search text is non-empty) bypass the tree and render as the existing flat
  ranked list — search shouldn't force the user to expand folders.

### AC-2: Editor wrap is bound to the pane and toggleable
- The raw editor and live-preview block editor both fit their content to the central panel's
  current width.
- A user preference `ui.editor_word_wrap: bool` (default `true`) controls wrap behavior:
  - `true`: long lines soft-wrap at the pane edge; no horizontal scroll bar.
  - `false`: long lines do not wrap; the editor scrolls horizontally.
- The toggle lives in the right panel under "Editor" (new collapsing section), persisted in
  per-folder settings JSON. Per-folder is the v1 scope; an app-global override can come later if
  needed.
- The setting takes effect on the very next frame (no app restart).

### AC-3: Outline renders styled headings
- Each heading row uses egui's `RichText` with a font size proportional to the level:
  - H1 → 1.25× base font, weight bold
  - H2 → 1.15× base, bold
  - H3 → 1.05× base, semibold
  - H4 → 1.00× base, semibold
  - H5 → 0.92× base, regular
  - H6 → 0.86× base, regular
  (Numbers tunable; intent: H1 noticeably larger, H6 noticeably smaller.)
- The displayed text strips `#` prefix and any leading/trailing `**`, `__`, `*`, `_` emphasis
  markers, and trims whitespace. Internal emphasis markers in the heading text are also stripped
  for the outline display only (the source note is unmodified).
- Each row is left-aligned and has no bullet/colon prefix.
- Indent by `(level - 1) * 8 px` to give an "outline" feel.
- Clicking a heading row jumps the editor cursor / scroll to that heading in the central pane
  (stretch — not blocking AC-3 if it's costly).

### AC-4: Graph uses a force-directed layout
- Replace the current circular layout in `draw_graph` with a force-directed layout
  (Fruchterman–Reingold or equivalent — no external dep, ~80–120 lines of Rust):
  - Initial positions seeded by golden-angle spiral or random.
  - Iterate `N` simulation steps (e.g. 200) on first paint after the graph snapshot changes,
    then leave positions stable until the next index update.
  - Repulsion between all node pairs; attraction along edges; cooling schedule.
- Pan via primary-button drag on empty space; zoom via scroll wheel (or pinch).
- Node radius scales with degree (more-linked = bigger). Cap visual size.
- Labels: render only when zoomed past a threshold *or* on hover. At default zoom, dense regions
  show only nodes — no overlapping label soup.
- The selected note is highlighted (different fill color and a 1px halo) and centered on the
  first frame after selection changes.
- Click on a node selects that note (drives `folder.selected`).

## Design Decisions (with overridable defaults)

- **Tree expansion state location**: `<folder>/.sideromelane/settings.json` under
  `ui.tree_expanded_paths`. *Override:* could move to app-local state if you'd rather keep view
  state out of folder metadata.
- **Word-wrap setting scope**: per-folder (matches existing settings model). *Override:* app-global
  if you want this to follow the user across folders.
- **Bold/italic stripping in outline**: applied to the displayed string only; source unchanged.
  *Override:* render the markup faithfully (then we keep the `**` and `*`).
- **Graph algorithm**: hand-rolled Fruchterman–Reingold to avoid pulling in a graph crate.
  *Override:* if you want something more sophisticated (`force_graph`, `petgraph` layouts), say so
  and I'll request approval.
- **Click-to-jump in outline (AC-3 stretch)**: included as stretch to avoid blocking the core fix.
  *Override:* require it before merge.

## Implementation Notes

Files most affected:

- `crates/sideromelane-app/src/main.rs` — `left_panel` (rewrite for tree), `right_panel`
  (outline rendering and editor settings UI), `main_panel` (wrap toggle wiring), `draw_graph`
  (replace algorithm).
- `crates/sideromelane-core/src/folder_settings.rs` — extend schema with `UiSettings`:
  ```rust
  pub struct UiSettings {
      pub editor_word_wrap: bool,         // default true
      pub tree_expanded_paths: Vec<String>, // default empty
  }
  ```
  Add as a new field on `FolderSettings`. `serde(default)` so older settings files still load.
- `crates/sideromelane-core/src/analysis.rs` — keep heading text un-stripped in `Heading::text`.
  Stripping is purely a rendering concern in the app crate.

New helpers (likely in `crates/sideromelane-app/src/`):

- `tree.rs` — pure builder turning a sorted `Vec<NoteId>` into a `TreeNode { name, children:
  Vec<TreeNode>, leaves: Vec<NoteId> }` for rendering.
- `outline.rs` — `fn display_heading_text(raw: &str) -> String` that strips emphasis markers, plus
  `fn heading_font_size(level: u8) -> f32` and `fn heading_weight(level: u8) -> egui::FontWeight`.
- `graph_layout.rs` — `fn fruchterman_reingold(nodes, edges, bounds, iterations) -> Vec<Pos2>`
  pure function, unit-testable.
- `graph_view.rs` — egui paint code that reads positions, handles pan/zoom (a `GraphViewState`
  that the app holds), and dispatches click selection.

## Testing Strategy

- `tree.rs`: unit tests on the builder against fixtures from
  `crates/sideromelane-core/tests/fixtures/folders/duplicate-stems/` and `valid/` (already exist).
- `outline.rs`: unit tests for the strip helper covering `**bold**`, `*italic*`, `__under__`,
  mixed `## **foo**`, no markers, only markers (degenerate empty string).
- `graph_layout.rs`: unit test that running FR on a small known graph (e.g. a triangle, a star)
  converges to layouts whose edges are shorter than the average inter-node distance — sanity
  check, not pixel-perfect.
- `folder_settings`: extend round-trip test to cover the new `ui` field; add a test that loading
  an older settings file (no `ui` key) yields defaults and saving back preserves the legacy fields.
- Manual: open the same folder used to capture the screenshots; confirm tree expand/collapse,
  width-bound editor with both wrap modes, outline rendered with sizes, and a force-directed graph
  with pan/zoom.

## Boundaries

**Always:**
- Use stdlib + egui only for the FR algorithm and tree builder.
- Keep heading parsing in core unchanged; rendering policies live in the app crate.
- `just check` and `just audit` clean before each commit.
- Use lefthook hooks (`just install-hooks` already done at session start).

**Ask first:**
- Adding any new dependency (e.g. `petgraph`, `force_graph`).
- Persisting UI state outside the folder (e.g. an app-local config).
- Changing the public API surface of `core::FolderSettings` in a way that breaks the v1 schema
  (we'd bump `version` and add a migration).

**Never:**
- Modify a note's source text in response to outline rendering or wrap setting changes.
- Block the typing path on graph layout (FR runs once per index update, not per frame).
- Mute scan errors that already surface via `IndexerEvent::ScanFailed`.

## Out of Scope

- Drag-to-reorder folders or notes in the tree.
- Folder-level or file-level icons beyond a chevron + plain text.
- Live-preview rendering of inline emphasis in the editor itself (separate work).
- Heading-click-to-jump (AC-3 stretch only).
- Graph filters (by tag, by recency) — Obsidian has these; we can add later.
- Tag pseudo-nodes in the graph (Obsidian shows `#kubernetes` as a node). Out of scope for this
  spec; lift if/when we implement tag indexing.

## Verification

When implemented:

1. `cargo run -p sideromelane-app --release` and open a real folder with nested directories.
2. AC-1: collapse and expand a folder; close the app; reopen; verify the expanded set persists.
3. AC-2: drag the central pane wider/narrower; toggle wrap; confirm horizontal scroll appears
   when wrap is off and disappears when wrap is on.
4. AC-3: open a note with H1–H4 headings; outline shows decreasing font sizes; no `##` or `**`
   visible.
5. AC-4: graph view shows clusters and hubs; drag empty space to pan; scroll to zoom; click a
   node to select; selected note highlighted and centered.
6. `just check` and `just audit` clean.

## Implementation Order (suggested)

To keep changes reviewable and to land safety-critical changes first:

1. AC-2 word-wrap (smallest UI surface, lowest risk; sets up `UiSettings` schema).
2. AC-3 outline sizing (pure rendering change; testable in isolation).
3. AC-1 files tree (medium surface; new `tree.rs` module + `left_panel` rewrite).
4. AC-4 graph layout (largest surface; new `graph_layout.rs` and `graph_view.rs`).

Each lands as its own commit so we can revert any one without affecting the others.
