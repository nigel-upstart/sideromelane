# ADR 0007: Folder Ignore Rules and Per-Folder Settings

## Status

Accepted

## Context

Sideromelane opens an arbitrary user-selected folder and walks every Markdown file
beneath it. Two product needs surfaced during early use:

1. Real folders contain build outputs, vendored dependencies, and tooling
   metadata (`node_modules/`, `target/`, `.git/`, `.obsidian/`) that the app must
   not index.
2. The walker behavior should be configurable per folder and survive when a
   folder is moved or copied between machines.

The existing walker was an inline recursive `read_dir` call that followed
symlinks, walked dotfolders, and had no way to express ignore rules.

## Decision

Adopt the `ignore` crate to power folder traversal and add a per-folder settings
file.

- Ignore inputs:
  - A `.sideromelaneignore` file at the folder root, with `.gitignore`-style
    glob syntax. Always loaded.
  - Optionally `.gitignore`, behind a per-folder toggle.
- Defaults are conservative:
  - Dotfiles and dotfolders are skipped.
  - Symlinks are not followed.
  - Walk depth is capped at 64.
  - The app-owned `.sideromelane/` directory is always excluded, even when
    dotfile inclusion is on.
- Per-folder settings live at `<folder>/.sideromelane/settings.json`.
  - Schema: `{ version: 1, ignore: { honor_gitignore, include_dotfiles, extra_globs } }`.
  - Unknown fields are preserved on load and round-tripped on save so newer
    builds can extend the schema without older builds destroying data.
  - Writes are atomic: payload is staged into `settings.json.tmp`,
    `sync_data()`-ed, renamed into place, then the parent directory is fsynced
    on a best-effort basis. This previews the safe-write story Cohort C will
    generalize to Markdown files.

Storing settings inside the folder keeps it self-describing: copying or syncing
the folder carries the user's preferences with it. This trades a small amount
of "files outside the app" purity for the ergonomics of folder portability,
mirroring how `.obsidian/` works in Obsidian. The settings file never touches
any `.md` content.

## Consequences

- Users can tame large folders by adding a `.sideromelaneignore` without code
  changes.
- The default walker can no longer be tricked into descending into `target/`
  loops or following symlinks out of the folder root.
- A new dependency (`ignore` plus its transitive crates) is on the supply-chain
  surface; all crates are already covered by the workspace `cargo-deny`
  allow-list.
- Future per-folder preferences (theme, editor mode defaults, embedding model
  knobs) get a place to live without inventing a new file.
- The atomic-write helper in `core::folder_settings` is intentionally
  duplicated with the safe-write primitive Cohort C lands in the app crate;
  they will be unified once the indexer/safe-write refactor is complete.
