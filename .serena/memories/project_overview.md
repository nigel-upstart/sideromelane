# Sideromelane — Project Overview

Sideromelane is a local-first Markdown note-taking desktop app (macOS) inspired by Obsidian. It provides:
- A folder-based note library with live Markdown preview
- Wiki-link (`[[Note Name]]`) graph visualization
- Full-text + semantic hybrid search
- Auto-save and conflict resolution

**Tech stack:** Rust, two crates:
- `crates/sideromelane-core` — pure library: parsing, analysis, indexing, search, graph
- `crates/sideromelane-app` — egui desktop GUI (eframe/egui)

**Build tool:** `just` (Justfile at repo root)
**Package manager:** Cargo

## Key commands
- `just check` — fmt-check + clippy + tests + cargo doc (run before every commit)
- `just audit` — cargo deny + cargo machete + typos + taplo fmt --check
- `just fmt` — auto-format
- `just test` — cargo test --workspace --all-features
- `just package` — build macOS .app bundle
- `cargo run -p sideromelane-app --release` — run the app

## Codebase structure
- `crates/sideromelane-core/src/` — analysis.rs, note.rs, index.rs, search.rs, lib.rs
- `crates/sideromelane-core/tests/` — integration tests per domain (folder_index, note_analysis, search_index, hybrid_search, etc.)
- `crates/sideromelane-app/src/` — main.rs, graph_view.rs, editor.rs, autosave.rs, conflict.rs, indexer.rs, etc.
- `docs/specs/` — numbered specs (0001, 0002, 0003…)
- `docs/adr/` — architecture decision records
