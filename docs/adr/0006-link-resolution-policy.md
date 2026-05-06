# ADR 0006: Link Resolution Policy

## Status

Accepted

## Context

Sideromelane resolves `[[Note Name]]` wiki links to actual notes at index time. Several
edge cases need an explicit policy: notes with identical file stems in different folders,
links to notes that do not exist, and the alias (`|`) and anchor (`#`) syntax common in
Obsidian-style vaults.

## Decision

- **Resolution key.** `[[Note Name]]` resolves by case-sensitive comparison against
  `Path::file_stem` — the filename without extension. Titles and full filenames with
  extension are not used as resolution keys in v1.

- **Ambiguous targets.** When two or more notes share the same stem, the link is
  considered ambiguous. The match is excluded from graph edges and backlinks. The conflict
  set is surfaced via `FolderIndex::ambiguous_targets()` so the UI can warn the user.
  Silently picking one of the matches is not acceptable in v1.

- **Missing targets.** Links whose target does not match any note stem are preserved in
  `NoteAnalysis::wiki_links()` and are left unresolved. They are available for future
  "create new note" workflows but produce no graph edge or backlink.

- **Alias and anchor.** `WikiLink` carries `alias: Option<String>` and
  `anchor: Option<String>`. They are parsed from `[[Target#anchor|alias]]` syntax and
  preserved for display and future deep-linking. Neither influences v1 resolution; only
  `target` is used when looking up a note.

## Consequences

- The resolution algorithm is deterministic and free of silent data loss.
- Users with duplicate-stem folders receive a visible warning rather than a silently
  wrong backlink graph.
- Missing links are not discarded, enabling future "did you mean?" or "create note" UX.
- Alias and anchor round-trip through the index without affecting search or graph edges.
