# Spec 0003: Tags as First-Class Graph Nodes

## Status

Implemented (d6720fb, f6321ed).

## Objective

Tags should participate in the graph the same way wiki links do. Today the graph shows
`note → note` edges from `[[Note Name]]` links only. Frontmatter `tags: [a, b, c]` already
flow into the search index but never become graph nodes. Inline `#tag` mentions in the body
aren't extracted at all.

After this spec lands:

- Every tag mentioned by at least one note becomes a node in the folder graph.
- Each note has an edge to every tag it uses (frontmatter or inline).
- The neighborhood-scoped graph view (Spec 0002 AC-2) shows tag nodes alongside note
  nodes; clicking a tag node refocuses the graph on it (revealing every note that uses
  that tag).
- Inline `#kubernetes` in the body of a note is functionally identical to listing
  `kubernetes` in the frontmatter `tags:` array.
- The search index treats inline and frontmatter tags as one set.

This brings the graph closer to the Obsidian-style behavior the user already pointed at
in Spec 0002 (where `#kubernetes` was a graph hub).

## Acceptance Criteria

### AC-1: Inline tag extraction
- `core::analysis` gains `extract_inline_tags(body: &str) -> Vec<Tag>` that returns every
  inline tag in document order, deduplicated.
- Syntax: `#` followed by one or more characters from `[A-Za-z0-9_-/]`. The `/` allows
  nested tags like `#kubernetes/storage`, which is how Obsidian and several other tools
  spell tag hierarchy.
- Whitespace rule: `#` must be preceded by start-of-string or whitespace. `# Heading` is
  not a tag (because of the space after `#`); `#tag` is. `text#middle` is not a tag
  (no leading whitespace).
- Trailing punctuation is excluded: `#tag.` produces tag `tag`. Punctuation set:
  `.`, `,`, `;`, `:`, `!`, `?`, `)`, `]`, `}`, `"`, `'`.
- Fenced code blocks (` ``` ` and `~~~`) are skipped — reuse the existing
  `analysis::non_fence_ranges`.
- Inline code spans (single backtick) are also skipped (extends the fence scanner to
  recognise inline backticks; this is a small extension since wiki-link extraction has
  the same gap today, called out in spec).

### AC-2: Tag domain type
- New public type in core: `pub struct Tag(String)` with:
  - `Tag::new(name: impl Into<String>) -> Result<Self, TagError>` — validates the
    syntax above and trims a single leading `#` if present, so callers can feed either
    `"#foo"` or `"foo"`.
  - `pub fn name(&self) -> &str` — returns the tag without the leading `#`.
  - `Display`, `Hash`, `Eq`, `PartialOrd`, `Ord`.
- `Frontmatter::list("tags")` is wrapped by a new `core::Frontmatter::tags()` helper
  that returns `Vec<Tag>`, validating each entry and dropping invalid ones (frontmatter
  is untrusted; we don't want a single bad tag to drop the whole list).

### AC-3: NoteAnalysis carries inline tags
- `NoteAnalysis` gains `inline_tags: Vec<Tag>` and `pub fn inline_tags(&self) -> &[Tag]`.
- `core::analysis::merged_tags(note: &MarkdownNote, analysis: &NoteAnalysis) -> Vec<Tag>`
  returns the union of frontmatter and inline tags, deduplicated, sorted, source-of-truth
  for downstream consumers.

### AC-4: FolderIndex exposes tag→notes
- `FolderIndex` gains `pub fn tag_index(&self) -> &BTreeMap<Tag, Vec<NoteId>>`.
- Built once during `FolderIndex::from_notes` by walking each note's `merged_tags` set.
- Tags with zero matched notes are not present (empty entries pruned).

### AC-5: Tags become graph nodes
- `core::index::Graph` gains a second node kind. The cleanest representation is a tagged
  enum:
  ```rust
  pub enum GraphNode {
      Note { note_id: NoteId },
      Tag { tag: Tag },
  }
  ```
- Existing call sites that read `node.note_id()` get a new `pub fn as_note(&self) -> Option<&NoteId>` helper or `match` arms; since `GraphNode` is currently a struct, this is a breaking change to the core API. ADR will record the migration; only `app::graph_view` consumes `GraphNode` today.
- `Graph::edges` gains tag-edges: for each note × tag pair, an edge from the note to the tag
  node. Direction: `note → tag` (the note "uses" the tag).
- `FolderIndex::neighborhood(focus, depth)` continues to BFS over both edge directions, so
  a tag with depth 1 returns every note that uses it; a note with depth 1 returns its
  wiki-link neighbors *and* its tags.

### AC-6: Graph view renders tags differently
- In `app::graph_view`, tag nodes use:
  - Slightly smaller default radius than notes.
  - A distinct fill color (e.g. soft purple matching the Obsidian convention) to set
    them apart at a glance.
  - Label prefix `#` so the user knows it's a tag.
  - Click selects the tag — switches `graph_view::current_focus` to the tag node and
    rebuilds the neighborhood. This shows every note using that tag plus, at depth ≥ 2,
    sibling tags they share.

### AC-7: Search treats inline tags as tags
- `app::indexer` and `core::search` already filter on `with_tag(...)`. Wire `merged_tags`
  into the searchable-tags pipeline so a query of `tag:kubernetes` matches notes that
  mention `#kubernetes` inline even if the frontmatter omits it.
- No changes to the lexical scoring weights; tags continue to count once per note in the
  ranking.

### AC-8: Tag click in editor → filter search
- Out of scope for this spec — defer. We can add a "click `#tag` in live preview to
  filter search" feature later. This spec is graph-only.

## Design Decisions (overridable)

- **Tag character set**: `[A-Za-z0-9_-/]`. *Override:* drop `/` (no nested tags) or
  add Unicode word characters via `is_alphanumeric` to support non-Latin scripts.
- **Trailing punctuation set**: `.,;:!?)]}"'`. *Override:* widen or narrow.
- **Tag node color**: soft purple. *Override:* pick another (must be visually distinct
  from the selected-note red and the regular-note blue).
- **`Graph::nodes` API break**: introduce the enum + migrate call sites in this PR.
  *Override:* keep `GraphNode` as a struct with a `kind: GraphNodeKind { Note, Tag }`
  field — less idiomatic but no breaking enum.
- **Inline-code-span skipping**: include in this spec. *Override:* defer to a future
  parser pass and only handle fenced blocks (matches today's wiki-link behavior).
- **Validation policy on frontmatter `tags:`**: drop invalid entries silently. *Override:*
  surface a status-bar warning per malformed entry.

## Implementation Notes

Files most affected:

- `crates/sideromelane-core/src/analysis.rs` — `extract_inline_tags`, `merged_tags`,
  extend `non_fence_ranges` to skip inline code spans.
- `crates/sideromelane-core/src/note.rs` — `Frontmatter::tags()` helper, `Tag` struct.
- `crates/sideromelane-core/src/index.rs` — `GraphNode` becomes an enum; `Graph::edges`
  gains note→tag edges; `tag_index` accessor; `neighborhood` adapts.
- `crates/sideromelane-core/src/search.rs` — `searchable_tags` reads `merged_tags` so
  inline tags participate in lexical filters.
- `crates/sideromelane-core/src/lib.rs` — new public exports.
- `crates/sideromelane-app/src/graph_view.rs` — rendering for tag nodes (color, label
  prefix, smaller radius, click handling for tag focus).
- `crates/sideromelane-app/src/main.rs` — adapt `select_note` etc. to the enum-based
  graph nodes; allow tag-focus through the same neighborhood pipeline.

New ADR:
- `docs/adr/0015-tags-as-graph-nodes.md` — the enum break, the inline-tag dialect, the
  decision to drop invalid frontmatter tags silently.

## Implementation Order (suggested)

1. `Tag` domain type + `Frontmatter::tags()` helper. Pure core, exhaustively tested.
2. `extract_inline_tags` + inline-code-span skip in `non_fence_ranges`. Pure core.
3. `merged_tags` + `tag_index` on `FolderIndex`. Pure core.
4. `GraphNode` enum migration + `note → tag` edges. Breaking change in core; touches
   `app::graph_view`.
5. `app::graph_view` rendering for tag nodes (color, prefix, click handling).
6. ADR 0015.

Each lands as its own commit. Existing test count must not decrease; targets land with
their own tests.

## Testing Strategy

Per acceptance criterion:

- **AC-1 inline extraction** (`analysis.rs::tests`): plain `#tag`, nested `#a/b`, mid-word
  `text#nope`, in fence ` ```\n#nope\n``` `, in inline code `` `#nope` ``, with trailing
  punctuation `#tag.` and `#tag,`, multiple tags on one line, hyphen+underscore,
  start-of-string vs preceded-by-newline.
- **AC-2 Tag**: round-trip `Tag::new("#foo") == Tag::new("foo")`, validation rejects
  `"#"`, `""`, `"foo bar"`, `"foo#bar"`, accepts `"foo/bar"`.
- **AC-3 NoteAnalysis**: a fixture note with both frontmatter and inline tags returns
  the deduplicated union via `merged_tags`.
- **AC-4 tag_index**: from a 3-note fixture with overlapping tag sets, assert the map
  shape and ordering.
- **AC-5 graph nodes**: a 2-note fixture with a shared tag produces nodes
  `{Note, Note, Tag}` and edges `{n1 → t, n2 → t}`. `neighborhood(tag, 1)` returns both
  notes plus the tag itself.
- **AC-6 rendering**: limited to compile-time correctness + a smoke that the click
  callback fires `select_focus(Tag(...))`. Visual rendering is manually verified.
- **AC-7 search**: a note with `#kubernetes` inline only (no frontmatter) is returned by
  `SearchQuery::tag("kubernetes")`.

## Boundaries

**Always:**
- Inline tag extraction skips fenced code and inline code spans.
- `Tag::new` validates input — no untrusted string ends up as a `Tag` payload.
- `merged_tags` is the single source of truth for downstream consumers; nothing reads
  `frontmatter.list("tags")` and `analysis.inline_tags()` separately.
- `just check` and `just audit` clean per commit.

**Ask first:**
- Adding a regex-style tag matcher dependency. Default plan: hand-rolled parser, no new
  deps.
- Changing tag dialect after this spec lands.

**Never:**
- Treat `# Heading` as a tag (whitespace rule).
- Index a literal `#` (zero-length tag) as a tag node.
- Allow user input through `Tag::new` without validation.

## Out of Scope

- Tag pages (virtual notes showing all tag matches).
- Tag autocompletion in the editor.
- Tag rename/refactor across notes.
- Tag aliases / synonyms.
- Tag color customisation per tag (one color for all tags in v1).
- Click `#tag` in live preview → filter search (future work).

## Verification

When implemented:

1. `cargo run -p sideromelane-app --release` against a real folder containing both
   frontmatter tags (`tags: [celery, datadog]`) and inline tags (`See #sumologic for
   context`).
2. The graph view (Cmd-G) on a tagged note shows tag nodes alongside its wiki-link
   neighbors. Tags are visually distinct.
3. Clicking a tag node refocuses the graph on it; the neighborhood is the set of notes
   using that tag.
4. Search field with `tag:datadog` returns notes regardless of whether `datadog` is in
   frontmatter or inline.
5. `just check` and `just audit` clean.
