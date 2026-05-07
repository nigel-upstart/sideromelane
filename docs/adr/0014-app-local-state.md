# ADR 0014: App-Local Persistent State

## Status

Accepted

## Context

Spec 0002 (Slice 3) calls for the app to remember which folder and note
the user had open across launches, and to expose a Preferences window
for app-wide preferences (startup behavior, default folder for fresh
sessions, auto-save debounce, default word-wrap). These preferences are
explicitly *not* per-folder: they describe how the app shell behaves
before any folder is chosen, so they cannot live in
`<folder>/.sideromelane/settings.json` (ADR 0007). They also do not
belong inside any single notes folder — copying a notes folder to
another machine should not carry your "open last folder" choice with it.

A second motivation is the Recent Folders LRU and the
left-pane splitter ratio added by later slices: both want a small
serialized blob persisted somewhere outside any opened folder.

## Decision

Add an app-local state document at
`<dirs::data_local_dir()>/sideromelane/state.json`, owned by the
`sideromelane-app` crate in a new `state` module. The schema mirrors
the strict-version pattern in `core::folder_settings`:

- `version: u32` — current value `1`. Documents whose `version` is
  greater than the current build's `CURRENT_STATE_VERSION` are rejected
  at load time so a downgrade cannot silently drop fields the older
  binary does not understand.
- `startup_mode`, `default_folder`, `last_folder`, `last_note`,
  `recent_folders`, `left_pane_split_ratio`, `auto_save_debounce_secs`,
  `default_word_wrap` — backing storage for the spec fields.

Writes go through a new `io::safe_write_bytes` sibling to the existing
`safe_write` primitive (ADR 0009). The two share the same
temp-file + `sync_data` + rename + best-effort parent-fsync algorithm;
the new function takes arbitrary bytes and uses a generic
`<filename>.tmp` temp path rather than the `.md.tmp` extension and
symlink guard the user-facing notes path needs. This keeps the safe
algorithm in one place without forcing JSON writers through a `&str`
detour.

Saves are debounced 250 ms behind a dirty flag drained inside
`update`. The flag is set whenever `open_folder` runs (LRU push +
dedup), the selected note changes (last_note tracking), or the
Preferences window reports an edit. Errors are surfaced in the status
bar and the dirty flag is left set so a future frame retries.

Out-of-range values are clamped on load (`left_pane_split_ratio` to
`[0.1, 0.9]`, `auto_save_debounce_secs` to `[1, 60]`,
`recent_folders` to a hard cap of 10). This tolerates hand-edited or
older documents without rejecting them outright.

## Consequences

- Two distinct settings stores now exist: per-folder
  (`<folder>/.sideromelane/settings.json`, ADR 0007) and app-local
  (`state.json`). The split is intentional and stable.
- The `dirs` crate is added to the app crate's dependency surface to
  resolve the platform-specific data-local directory. `serde` and
  `serde_json` are also pulled into the app crate (they already lived
  in core).
- A corrupt `state.json` falls back to defaults rather than blocking
  app launch — the failure mode is "lose your last-folder pointer",
  not "cannot start the app".
- Slices 4–6 of spec 0002 build on this state plumbing without
  inventing a second persistence path.
