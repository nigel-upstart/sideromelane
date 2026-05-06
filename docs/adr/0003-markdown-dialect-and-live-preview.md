# ADR 0003: Markdown Dialect And Live Preview

## Status

Accepted

## Context

The product uses Markdown files as source of truth and adds Obsidian-style wiki links:

- `[[Note Name]]`
- `![[image.png]]`

The first UI must support raw editing and live preview, where inactive blocks render and the active
block remains editable Markdown source. Full CommonMark/GFM rendering can be improved over time, but
v1 needs a deterministic, local, testable behavior first.

## Decision

Use a small internal Markdown block model for v1 live preview and core indexing.

The supported v1 blocks are:

- YAML frontmatter block.
- ATX headings.
- Paragraphs.
- Lists and task list lines.
- Tables as preformatted Markdown rows.
- Fenced code blocks.
- Wiki links and wiki image embeds.

Keep the core parser dependency-free for now. If full CommonMark rendering becomes necessary, use
`pulldown-cmark` as the preferred parser because it is a Rust CommonMark pull parser with optional
GitHub-flavored table and task-list support.

## Consequences

- Live preview can be shipped without adding a browser or HTML renderer.
- The source file remains authoritative and easy to round-trip.
- Rendering is intentionally conservative; unsupported Markdown remains readable source text.
- A future `pulldown-cmark` integration should be added behind tests before replacing the internal
  renderer.
