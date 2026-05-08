# Plan — Sideromelane Current State & Next Work

_Updated 2026-05-08_

---

## State of play

### Completed

| Commit | Description |
|--------|-------------|
| `d6720fb`, `f6321ed` | Spec 0003: tags as first-class graph nodes (core + app) |
| `a04878a` | Fix graph view: circular initial seeding, force-directed pre-settle |
| `80c3fcf`, `e8c5367`, `6a73757` | Graph view tests + ship-review hardening |
| `584bcc8` | Spec 0005: Live Preview word-wrap toggle now works |

`just check` and `just audit` are clean on `main`.

---

## Pending manual verifications

Both require running `just package && open target/package/Sideromelane.app`
against a real notes folder with inline tags and links.

### Spec 0003 — Tags in graph view
- [ ] Graph view (Cmd-G): tag nodes visible, soft purple, `#`-prefix label
- [ ] Click tag node → neighborhood rebuilds around that tag
- [ ] Search `tag:datadog` → matches notes with inline `#datadog` only

### Spec 0005 — Live Preview word wrap
- [ ] Word wrap OFF: Live Preview text does not wrap; pane scrolls horizontally
- [ ] Word wrap ON: text wraps at pane width (no regression)

---

## Next feature: Spec 0006 — `[[` wiki-link autocomplete

**Why next:** Highest-value unimplemented "Should Have" in the product spec. Without it, creating inter-note links requires users to remember exact note names. Everything needed is already available: the `FolderIndex` holds all note stems and the active note source is in scope during editing.

### Scope

In both **Raw** mode and **Live Preview active blocks**, when the user types `[[`:
1. A popup appears listing matching notes from the folder index.
2. Typing more characters filters the list (case-insensitive prefix/substring match against note file stems).
3. Pressing Enter or clicking a row completes the link as `[[Note Name]]` (or `[[Note Name|` if the user typed an alias-start character).
4. Pressing Escape dismisses the popup without inserting.
5. The popup disappears automatically when the cursor moves outside the `[[...` span.

### Out of scope for v1 autocomplete
- Autocomplete for new (not-yet-existing) notes (link still accepted, just no suggestion)
- `#tag` autocomplete
- Fuzzy matching (prefix/substring is sufficient)
- Alias or anchor autocompletion

### Dependency graph

```
AC-1: detect [[  trigger position in source text    → independent
AC-2: filter note list by typed prefix              → depends on AC-1
AC-3: popup UI (egui popup/window, arrow-key nav)   → depends on AC-2
AC-4: insert completion into source text             → depends on AC-3
AC-5: dismiss on Escape / cursor-out-of-span        → depends on AC-3
AC-6: works in raw_editor                            → depends on AC-4, AC-5
AC-7: works in live_preview active block             → depends on AC-4, AC-5
```

### Implementation slices

**Slice 1 — Core detection helper** (no UI)
- `fn find_wiki_link_prefix(source: &str, cursor_byte: usize) -> Option<&str>`
  Returns the typed prefix if cursor is inside an open `[[...` (no closing `]]` yet), else `None`.
- Unit tests: cursor mid-prefix, cursor before `[[`, completed link, nested brackets.

**Slice 2 — Filtered completion list** (no UI)
- `fn complete_note_links<'a>(stems: &'a [&str], prefix: &str) -> Vec<&'a str>`
  Returns stems where `stem.to_lowercase().contains(prefix.to_lowercase())`, capped at 10.
- Unit tests: empty prefix returns all (capped), typed prefix filters, case-insensitive.

**Slice 3 — Popup widget** (egui)
- `struct WikiLinkPopup { items: Vec<String>, selected: usize }`
- Renders as an `egui::popup_above_or_below_widget` anchored to the TextEdit.
- Arrow keys move selection; Enter fires a callback with the chosen stem.
- Tests: selection wraps, escape returns None.

**Slice 4 — Wire into raw_editor**
- After `TextEdit` response, check for `[[` prefix at cursor.
- If found: compute filtered list from `folder_index`, show popup.
- On selection: splice `[[stem]]` into source replacing the `[[prefix` span.

**Slice 5 — Wire into live_preview active block**
- Same as Slice 4 but inside the `is_active` block TextEdit.

**Slice 6 — Integration & cleanup**
- Smoke test: raw_editor with a two-note folder, type `[[F`, assert popup shows "Focus".
- Update `tasks/todo.md` acceptance criteria.

### Acceptance criteria

- [ ] Typing `[[` in raw mode shows a note-name popup
- [ ] Typing `[[k` filters to notes whose stem contains "k" (case-insensitive)
- [ ] Enter/click inserts `[[Matching Note]]` at cursor
- [ ] Escape dismisses without inserting
- [ ] Popup disappears when cursor moves outside the `[[...` span
- [ ] Works in Live Preview active blocks
- [ ] `just check` passes

---

## Backlog (not yet planned)

- **Tabs** — open/close multiple notes simultaneously (explicit TODO stubs exist in `menu.rs`)
- **Graph controls** — zoom, pan, in-graph search, depth slider surfaced in UI
- **`[[` autocomplete** — create-new-note flow (typing a name that doesn't exist)
- **Tag autocomplete** — `#` trigger similar to `[[` autocomplete
