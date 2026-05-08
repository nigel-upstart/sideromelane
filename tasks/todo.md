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

- [ ] **Slice 1:** `find_wiki_link_prefix` helper + unit tests
- [ ] **Slice 2:** `complete_note_links` filtered list + unit tests
- [ ] **Slice 3:** `WikiLinkPopup` egui widget + tests
- [ ] **Slice 4:** Wire into `raw_editor`
- [ ] **Slice 5:** Wire into `live_preview` active block
- [ ] **Slice 6:** Integration smoke test + cleanup

### Acceptance criteria
- [ ] Typing `[[` in raw mode shows a note-name popup
- [ ] Typing `[[k` filters to notes whose stem contains "k" (case-insensitive)
- [ ] Enter/click inserts `[[Matching Note]]` at cursor
- [ ] Escape dismisses without inserting
- [ ] Popup disappears when cursor moves outside the `[[...` span
- [ ] Works in Live Preview active blocks
- [ ] `just check` passes
