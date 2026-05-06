# ADR 0002: egui Native App Shell

## Status

Accepted

## Context

Sideromelane needs a local-only packageable macOS desktop app with a fast typing path, a raw
Markdown editor, live preview, side panels, and a graph view. The project should stay Rust-first and
avoid choosing a WebView/Electron-style stack unless the product requires browser-only capabilities.

## Decision

Use `eframe`/`egui` for the first app shell.

The app shell will live in `crates/sideromelane-app` and depend on `sideromelane-core` for domain
logic. UI-specific filesystem orchestration is allowed in the app crate, while parsing, indexing,
search, backlinks, and graph models remain in the core crate.

Use `rfd` for native file/folder dialogs.

## Consequences

- The app remains Rust-native and avoids a JavaScript build chain.
- The first UI can draw the graph directly with egui painting primitives.
- Live preview is implemented as a block editor: inactive blocks render, and the active block stays
  editable Markdown source.
- Deep text-editor behavior may require future refinement, but the architecture avoids locking the
  core engine to a GUI framework.
