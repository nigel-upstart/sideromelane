# ADR 0015: Tags as Graph Nodes

## Status

**Accepted**

## Context

Spec 0003 promotes tags (frontmatter `tags: [...]` and inline `#tag` mentions) to
first-class graph nodes. Until now, `core::index::Graph` modeled only `note → note`
edges from `[[Wiki Link]]` references; tags were a search-index input only. After
this work, the graph contains both note nodes and tag nodes, with `note → tag` edges
for every tag a note uses.

`GraphNode` today is a struct:

```rust
pub struct GraphNode {
    note_id: NoteId,
}

impl GraphNode {
    pub fn note_id(&self) -> &NoteId { &self.note_id }
}
```

It is consumed by exactly one external module — `crates/sideromelane-app/src/graph_view.rs` —
which builds an `egui_graphs::Graph` from `FolderIndex::graph()`. There are no other
public consumers; the type is `pub` only because it leaks through the `Graph` accessor.

To represent two kinds of nodes (note and tag), we need to change `GraphNode`. The
question is *how*. Two shapes are reasonable.

## Decision (the question for you)

**Pick one of the two representations below for `GraphNode`.** The rest of Spec 0003
implementation is the same either way; the choice is purely about the public type
shape and the migration cost on the one consumer (`graph_view.rs`).

### Option A — `GraphNode` becomes an enum (recommended)

```rust
pub enum GraphNode {
    Note { note_id: NoteId },
    Tag  { tag: Tag },
}

impl GraphNode {
    pub fn as_note(&self) -> Option<&NoteId> { … }
    pub fn as_tag(&self)  -> Option<&Tag>    { … }
}
```

**Pros**
- Idiomatic Rust. Compiler forces every match site to handle both variants — adding a
  third node kind later (e.g. `Folder`, `Heading`) is a compile error in every
  consumer until handled, which is what we want.
- Read sites are clearer: `match node { Note { … } => …, Tag { … } => … }` reads as
  exactly what's happening.
- No "phantom" empty fields when one variant doesn't apply.

**Cons**
- One breaking change to the `pub` surface. `graph_view.rs` has ~6 call sites that
  read `node.note_id()` today; each becomes a `match` (or an `as_note()` call).
- Anyone outside this repo holding a `&GraphNode` would need to update — moot since
  the only consumer is internal.

### Option B — `GraphNode` stays a struct with a `kind` field

```rust
pub struct GraphNode {
    pub kind: GraphNodeKind,
}

pub enum GraphNodeKind {
    Note { note_id: NoteId },
    Tag  { tag: Tag },
}
```

**Pros**
- Less breaking. Existing `node.note_id()` accessor can stay (returning `Option<&NoteId>`
  via `match self.kind`), so consumers compile with one signature change.
- If we later add fields that apply to *all* node kinds (e.g. `degree: usize`,
  `display_label: String`) they live on `GraphNode` regardless of `kind`.

**Cons**
- Extra indirection at every read site (`node.kind.something()` instead of
  `match node { … }`).
- Adding a new variant to `GraphNodeKind` doesn't force consumers to handle it the way
  a top-level enum does — easier to silently drop a new node kind in rendering.
- The `kind` field is the only thing on `GraphNode` for the foreseeable future, so the
  outer struct adds no information today.

## Recommendation

**Option A (enum).** Reasons:
1. The single consumer is internal and small (~6 call sites). Migration cost is low.
2. Future graph-node kinds (folders, headings, attachments) become exhaustiveness
   errors, which is the strongest defense against silently-broken rendering.
3. We don't have any "applies-to-all-nodes" fields planned. If we later do, the enum
   can grow a wrapper struct then.

The recommendation is reversible with one small refactor; the choice is not irreversible
either way.

## Consequences

**If Option A:**
- `graph_view.rs` rewrites the ~6 `node.note_id()` reads as `match node { GraphNode::Note { note_id } => …, GraphNode::Tag { tag } => … }` (or `as_note()` / `as_tag()` shortcuts).
- `FolderIndex::neighborhood` continues to BFS over edges; the result struct gains a
  `tags: Vec<Tag>` field alongside `notes: Vec<NoteId>`, OR keeps a single
  `Vec<GraphNode>` — to be decided in implementation, not blocking this ADR.
- Tag rendering in `egui_graphs` uses a different shape/color (purple, smaller radius,
  `#`-prefixed label).

**If Option B:**
- `node.note_id()` becomes `Option<&NoteId>` and rendering gates on `Some(_)`. Tag
  rendering keys off `match node.kind`. Same downstream visual outcome.

## Other Spec 0003 dialect decisions (separate, less weighty)

These are not blocking ADR 0015 but are recorded here so the spec doc has a single
home for the choices:

- **Inline tag character set**: `[A-Za-z0-9_-/]` — supports nested `#kubernetes/storage`.
- **Trailing punctuation stripped from inline tags**: `.,;:!?)]}"'`.
- **Frontmatter `tags:` validation**: invalid entries dropped silently.
- **Tag node color**: soft purple, smaller radius than note nodes.
- **Inline code span skip**: extend `non_fence_ranges` to also skip single-backtick
  spans (closes a parallel hole that wiki-link extraction has today).

If you want any of these flipped, say so when you answer Option A vs B and I'll bake
both into the implementation pass.
