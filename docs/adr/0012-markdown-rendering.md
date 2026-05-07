# ADR 0012: Markdown Rendering Via egui_commonmark

## Status

Accepted

## Context

ADR 0003 deferred a real CommonMark renderer in favor of a hand-rolled block model
(`render_block` / `markdown_blocks` / `preview_text` in `crates/sideromelane-app/src/main.rs`).
That renderer was conservative on purpose: it shipped without a parser dependency and kept
unsupported Markdown legible as raw source. As Spec 0002 expanded the live-preview surface
(tables, task lists, fenced code with highlighting, inline images), the cost of extending
the hand-rolled renderer began to outweigh the cost of taking a parser dependency.

`egui_commonmark` is a small egui-native CommonMark viewer backed by `pulldown-cmark`. It
ships with image loading via `egui_extras`, optional syntax highlighting via `syntect`, a
simple `CommonMarkCache` for cross-frame state, and a `link_hooks` registry that lets the
host intercept specific URLs.

The product still needs Obsidian-style wiki links (`[[Note]]`, `[[Note|Alias]]`,
`[[Note#anchor]]`, `[[Note#anchor|Alias]]`) and image embeds (`![[image.png]]`), neither of
which is part of CommonMark.

## Decision

Use `egui_commonmark` (0.23, default features) to render inactive live-preview blocks.
Wiki links and image embeds become a display-only pre-pass in
`crates/sideromelane-app/src/preview.rs` that rewrites them to standard CommonMark before
handing the text to the viewer. The pre-pass:

- skips fenced code blocks using a small local CommonMark fence scanner that mirrors
  `sideromelane_core::analysis::non_fence_ranges` (kept local to avoid widening the core
  crate's public surface for a single caller);
- maps `[[Note]]` / `[[Note|Alias]]` / `[[Note#anchor]]` / `[[Note#anchor|Alias]]` to
  Markdown links of the form `[Display](sideromelane://note/Note[#anchor])`;
- maps `![[image.png]]` to a `file://` image link rooted at `<folder>/assets/`.

Each `sideromelane://note/...` URL is registered with `CommonMarkCache::add_link_hook` so
the viewer routes clicks through the cache instead of opening the OS browser. The app
polls the registered hooks after each render and stores the target on
`SideromelaneApp::pending_link_click`, which `main_panel` drains to select the target
note. The source-of-truth on disk is unchanged.

The active block in live preview keeps using a raw `TextEdit::multiline`, so editing
behavior is identical to before.

## Consequences

- Live preview gains real CommonMark / GFM rendering (task lists, tables, fenced code with
  highlighting, inline images) without app-level work per feature.
- New runtime deps: `egui_commonmark`, `egui_commonmark_backend`, `pulldown-cmark`, plus
  `egui_extras`'s image loader. License footprint stays inside the existing allow list
  (`MIT` / `Apache-2.0`).
- Wiki-link and image-embed semantics remain owned by the app; the core indexer continues
  to use `sideromelane_core::analysis::non_fence_ranges` for the same cases. The two
  scanners are intentionally kept in sync but not unified across crates.
- Anchor navigation (`sideromelane://note/X#anchor`) is parsed but currently ignored on
  click — the target note is selected and the anchor is dropped. A follow-up can wire the
  anchor through the outline-jump path landed in Slice 2.
- Click handling depends on `egui_commonmark`'s `link_hooks` API; a hard fork would need
  another approach (e.g., intercepting `OutputCommand::OpenUrl`).

## Future Work

- Real anchor navigation that scrolls/focuses the matching heading.
- Inline image embed alt-text from the `[[image.png|Alt]]` form once the live-preview UI
  surfaces alt text.
- Reconcile the local fence scanner with the core helper if a third caller appears.
