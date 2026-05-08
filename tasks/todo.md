# Todo

## Manual verifications (requires running app)

### Spec 0003 — Tags in graph view
Run `just package && open target/package/Sideromelane.app` against a real notes folder:
- [ ] Graph view (Cmd-G): tag nodes visible, soft purple, `#`-prefix label
- [ ] Click tag node → neighborhood rebuilds around that tag
- [ ] Search `tag:datadog` → matches notes with inline `#datadog` only

### Spec 0005 — Live Preview word wrap
- [ ] Word wrap OFF: Live Preview text does not wrap; pane scrolls horizontally
- [ ] Word wrap ON: text wraps at pane width (no regression)

---

## Spec 0006 — `[[` wiki-link autocomplete

- [x] **Slice 1:** `find_wiki_link_prefix` helper + unit tests
- [x] **Slice 2:** `complete_note_links` filtered list + unit tests
- [x] **Slice 3:** `WikiLinkPopup` egui widget + tests
- [x] **Slice 4:** Wire into `raw_editor`
- [x] **Slice 5:** Wire into `live_preview` active block
- [x] **Slice 6:** Integration smoke test + cleanup

### Acceptance criteria
- [x] Typing `[[` in raw mode shows a note-name popup
- [x] Typing `[[k` filters to notes whose stem contains "k" (case-insensitive)
- [x] Enter/click inserts `[[Matching Note]]` at cursor
- [x] Escape dismisses without inserting
- [x] Popup disappears when cursor moves outside the `[[...` span
- [x] Works in Live Preview active blocks
- [x] `just check` passes

### Manual verification (requires running app)
- [ ] In raw mode: type `[[` → popup appears with note stems
- [ ] Type more chars → list filters case-insensitively
- [ ] Press Enter → `[[stem]]` inserted, popup closes
- [ ] Press Escape → popup dismisses without insertion
- [ ] Move cursor out of `[[...` span → popup disappears
- [ ] Same behavior in Live Preview active block
