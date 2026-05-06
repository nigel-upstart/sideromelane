# Spec: Sideromelane Local Graph Notes

## Phase

Phase 1: Specify.

This spec is a review draft. Implementation should not begin until the assumptions and open
questions are accepted or corrected.

## Assumptions

1. Sideromelane is a local-only native macOS desktop app built with Rust.
2. Markdown files are the durable source of truth; indexes and caches are rebuildable.
3. v1 must be packageable as a macOS app, but signing and notarization can be decided later.
4. The GUI framework is still undecided and must be selected through an ADR before app UI code is
   added.
5. Semantic search must work without required network calls. Any embedding model, runtime, or
   dependency must be local-capable and approval-gated.
6. Future extensions are intentionally out of scope for this spec.

## Objective

Build a packageable, shippable Markdown knowledge workspace for local notes. The app enables a user
to create, edit, browse, tag, link, search, and visualize Markdown notes stored in a local folder.

The primary user is someone who wants Obsidian-like core primitives without a plugin ecosystem,
theme system, sync service, or proprietary storage format.

Success means the user can open the app, start typing in an existing or new note immediately, and
let indexing, backlinks, graph data, and semantic search hydrate progressively in the background.

## Product Scope

### Must Have

- Local folder selection and file browsing for Markdown notes and image assets.
- Markdown note creation, editing, and safe persistence to disk.
- Two note modes:
  - Raw view: plain Markdown source, including YAML frontmatter.
  - Live preview: editable Markdown where inactive lines or blocks are rendered, while the active
    cursor line or block remains Markdown source.
- YAML frontmatter support for title, tags, status, and arbitrary simple fields.
- Internal wiki links using `[[Note Name]]`.
- Link resolution and navigation between notes.
- Backlinks panel that lists notes linking to the current note.
- Markdown rendering for headings, lists, checkboxes, links, tables, inline images, and code blocks.
- Image drag-and-drop into the folder assets area with insertion as `![[image.png]]`.
- Keyword search across file name, title, content, links, tags, and frontmatter.
- Local semantic search using embeddings, with background indexing and eventual consistency.
- Graph view where notes are nodes and links are edges.
- Three-panel layout:
  - Left: file explorer and search.
  - Main: editor, live preview, and optional tabs.
  - Right: backlinks, outline, and graph toggle.

### Should Have

- `[[` autocomplete for linking existing notes and creating missing notes.
- Outline panel generated from Markdown headings.
- Tabs for multiple open notes.
- Graph controls for zoom, pan, filtering, and in-graph search.

### Out Of Scope

- Plugin ecosystem.
- Theme customization.
- Real-time collaboration.
- Database/table engines.
- Required cloud services.
- Required network calls.
- External analytics or telemetry.
- AI-assisted writing, summarization, or generation.
- Dedicated read-only render view as a separate v1 mode.

## Tech Stack

- Language: Rust, stable toolchain.
- Package manager: Cargo.
- Workspace: Cargo workspace with `sideromelane-core` for pure domain logic.
- Desktop target: native macOS app.
- GUI framework: undecided; choose through an ADR after this spec is accepted.
- Storage: local filesystem for notes and assets. Derived indexes may use app-local cache storage
  once selected through implementation planning.
- Search: keyword index plus local embedding index. Exact crates, embedding model, and vector
  storage require approval before dependencies are added.
- Networking: none required for v1.

## Commands

```sh
just fmt
just fmt-check
just lint
just test
just doc
just check
just audit
just install-tools
just install-hooks
just hooks
```

Completion checks for Rust changes:

```sh
just check
```

Dependency or supply-chain changes also require:

```sh
just audit
```

## Project Structure

Current structure:

```text
crates/sideromelane-core/  Pure domain logic and deterministic tests.
docs/                      Project notes and architecture decisions.
docs/adr/                  Accepted architecture decision records.
skills/                    Imported agent-skills used by this repo.
scripts/                   Local developer automation.
```

Expected future structure after the GUI ADR:

```text
crates/sideromelane-core/      Folder, note, link, metadata, index, and graph domain logic.
crates/sideromelane-app/       Application orchestration and desktop shell boundary.
crates/sideromelane-macos/     Narrow macOS platform adapters, if needed.
tests/fixtures/folders/         Small sample folders for integration tests.
docs/adr/                      GUI, packaging, storage, and embedding decisions.
```

Do not add app/platform crates until the GUI/framework decision is accepted.

## Core Model

### Folder

A folder is a user-selected local root folder containing Markdown notes and assets. The app must keep
the folder directly usable outside Sideromelane.

The folder may also contain app-owned metadata that does not affect direct usability:

- `.sideromelaneignore` — optional `.gitignore`-syntax file at the folder root that excludes paths
  from indexing. See ADR 0007.
- `.sideromelane/settings.json` — per-folder settings (ignore behavior, dotfile inclusion,
  optional honoring of `.gitignore`). See ADR 0007.

Default scan behavior excludes dotfiles, dotfolders, and the `.sideromelane/` directory itself. The
defaults are user-toggleable per folder.

### Note

A note is a `.md` file with optional YAML frontmatter followed by Markdown content.

```md
---
title: Launch Plan
tags: [planning, product]
status: draft
---

# Launch Plan

Some content with a [[Related Note]].

![[image.png]]
```

### Metadata

Frontmatter is parsed as untrusted user-authored input. Supported v1 display fields are `title`,
`tags`, and `status`; other scalar or list fields may be displayed generically when parsing succeeds.

Raw view shows frontmatter as text. Live preview shows a structured metadata block when inactive and
switches the active metadata line or block back to editable YAML source while the cursor is inside
it.

### Links

Internal links use `[[Note Name]]`. Links resolve by case-sensitive comparison against note file
stems. Ambiguous matches (multiple notes sharing a stem) are surfaced via
`FolderIndex::ambiguous_targets()` for the UI to warn on, rather than silently picked. Missing-note
links are preserved and may be used to create a new note. Wiki links may carry an alias
(`[[Note|Display]]`) and an anchor (`[[Note#section]]`); both are preserved on the parsed link but
do not influence resolution in v1. See ADR 0006.

### Assets

Images are stored under a folder-local assets location. The default location should be predictable,
for example `assets/`, but the exact naming and collision policy must be specified before
implementation.

## Architecture

The app is organized around pure core services and narrow adapters:

```text
Desktop UI
  -> App orchestration
    -> Core engine
      -> Folder manager
      -> Markdown/frontmatter parser
      -> Link resolver
      -> Indexer
      -> Search service
      -> Graph builder
    -> Platform adapters
      -> Filesystem
      -> macOS packaging/runtime
```

Core behavior should be testable without a running desktop app. Platform-specific macOS behavior
must stay behind explicit module boundaries.

## Background Work

Startup must not block on indexing, embeddings, graph building, or search hydration. The app opens
to an editable surface first (one initial note read shallowly), and a background indexer worker
hydrates derived data eventually. See ADR 0008.

The indexer worker:

- Discovers Markdown files via `core::scan::walk_markdown_paths` honoring per-folder
  `.sideromelaneignore` and dotfile/`.gitignore` settings.
- Parses changed Markdown and frontmatter.
- Extracts links, tags, headings, and image references.
- Rebuilds keyword and semantic search indexes.
- Rebuilds backlink and graph data.

The UI side is purely advisory: search, backlinks, and graph reads run against whatever indexes the
app currently holds. They start empty, then become eventually consistent as `IndexUpdated` events
arrive. Partial results are expected while warmup is in progress.

## Performance Targets

- Typing-ready startup target: less than 75 ms after the app process is ready to show UI.
- Typing latency target: less than 16 ms per interaction under normal note size.
- Note open target: less than 100 ms for cached or already-read notes.
- Search input target: results begin updating immediately from available indexes.

These targets are product goals. Exact measurement harnesses should be defined once the GUI
framework is selected.

## Data Integrity

- Treat all folder files, frontmatter, Markdown, image names, IPC payloads, imported documents, and
  pasted text as untrusted input.
- Note writes are crash-safe: temp file → `sync_data` → rename → best-effort parent directory
  `sync_all`. See ADR 0009.
- Path components are required to round-trip through UTF-8; non-UTF-8 paths are rejected at
  `NoteId` construction time so derived indexes cannot diverge from on-disk content.
- Image drops are size-capped (32 MiB), magic-byte validated, and filename-sanitized before being
  copied into the folder's assets directory.
- Derived indexes must be rebuildable from the folder.
- App crashes must not corrupt Markdown files.
- No secrets, local user data, generated app bundles, or signing/notarization credentials belong in
  the repository.

## Code Style

Prefer small, explicit domain types and narrow interfaces.

```rust
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NoteId {
    relative_path: PathBuf,
}

impl NoteId {
    #[must_use]
    pub fn from_folder_relative_path(relative_path: PathBuf) -> Self {
        Self { relative_path }
    }

    #[must_use]
    pub const fn relative_path(&self) -> &PathBuf {
        &self.relative_path
    }
}
```

Conventions:

- Prefer domain names such as `Folder`, `Note`, `Frontmatter`, `WikiLink`, `Backlink`, and
  `GraphEdge`.
- Keep parsing, indexing, and filesystem effects separated.
- Return typed errors at boundaries instead of panicking.
- Avoid `unwrap` and `expect` outside tests unless the invariant is documented.
- Do not introduce `unsafe`.

## Testing Strategy

- Unit tests live near the code they verify.
- Integration tests live under each crate's `tests/` directory.
- Fixture folders should cover valid Markdown, malformed frontmatter, duplicate note names, broken
  links, image references, and large-enough notes to catch obvious latency regressions.
- Core tests should verify parsing, link extraction, backlink generation, metadata extraction,
  graph construction, search ranking inputs, and safe write behavior.
- App/UI/runtime checks should be added after the desktop framework is selected.
- New behavior should start with a failing test when practical.

## Boundaries

Always:

- Run `just check` before marking Rust changes complete.
- Keep Markdown files and assets readable outside the app.
- Keep indexing, embeddings, graph building, and search hydration off the startup critical path.
- Validate untrusted data at filesystem, parser, IPC, and UI boundaries.
- Put major architecture decisions in `docs/adr/`.

Ask first:

- Add any dependency.
- Choose or replace the GUI framework.
- Add network capability.
- Add or configure an embedding model/runtime.
- Choose persistent index/vector storage.
- Change packaging, signing, sandboxing, or notarization behavior.
- Change CI quality gates.
- Introduce `unsafe`.

Never:

- Commit secrets, local user data, generated app bundles, or signing/notarization credentials.
- Disable tests, lints, or security checks to make CI pass.
- Require a cloud service or network call for v1 functionality.
- Store notes in a proprietary source-of-truth format.
- Implement plugin, theme, sync, or collaboration systems in v1.

## Success Criteria

- A user can create and edit Markdown notes in a local folder.
- A user can switch between raw and live preview modes.
- In live preview, inactive Markdown lines or blocks are rendered, and the active cursor line or
  block remains editable Markdown source.
- YAML frontmatter is visible in raw mode and cleanly displayed as editable metadata in live preview.
- `[[Note Name]]` links resolve, navigate, and generate backlinks.
- Dragged images are copied into the folder and rendered inline as wiki image embeds.
- Keyword search works over note text, titles, tags, links, file names, and frontmatter.
- Semantic search returns local embedding-based matches without required network calls.
- Search and graph data hydrate in the background without blocking initial editing.
- Graph view displays note-link relationships and supports basic navigation controls.
- The app can be packaged as a local macOS desktop app through the selected app-shell strategy.
- `just check` passes for implemented Rust changes.
- Dependency or supply-chain changes pass `just audit`.

## Open Questions

1. ~~What is the accepted app-shell or GUI framework?~~ Resolved by ADR 0002 (eframe/egui).
2. Should the shippable v1 require signing and notarization, or is a local unsigned app bundle
   acceptable?
3. What is the default folder assets directory and collision policy for dropped images?
4. ~~How should duplicate note titles or duplicate stem names resolve?~~ Resolved by ADR 0006
   (ambiguous matches surfaced, not silently picked).
5. ~~What Markdown dialect is authoritative for tables, task lists, code fences, and wiki embeds?~~
   Resolved by ADR 0003 (internal v1 block model; pulldown-cmark deferred).
6. What local embedding model/runtime and vector storage are acceptable for v1? (ADR 0004 ships a
   hashed-token-vector baseline; a model-backed runtime requires its own ADR.)
7. What folder size should v1 performance targets be measured against?
