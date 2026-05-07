# ADR 0011: Native macOS Menu Bar Via muda

## Status

Accepted

## Context

Spec 0002 (Slice 5) calls for a real macOS menu bar so the app feels native:
File → Open Folder, Recent Folders, New Note, Save, Close; View → Show Graph,
Word Wrap; an application-menu Preferences entry. Toolbar buttons inside the
egui window already drive most of these actions, but they lack the
discoverability and global ⌘-key shortcuts macOS users expect — egui's
in-window key handling stops at the egui input layer, so shortcuts like ⌘O
fight with `TextEdit` focus.

`muda` (Tauri's "Menu Utilities for Desktop Apps") publishes an `NSMenu`
directly on `NSApplication`, gives us native accelerator handling that
survives focus changes, and ships its own `MenuEvent` channel keyed by stable
`MenuId`s. It is the same crate `tao` / `tauri` already use, so the
ecosystem is well-tested. The crate is dual-licensed `MIT OR Apache-2.0` —
inside our existing allow list.

## Decision

Add `muda` (0.19) with `default-features = false` (the defaults pull
`gtk`/`libxdo` for Linux, which we don't ship). Build the menu in a new
`crates/sideromelane-app/src/menu.rs` module that exposes:

- a `MenuAction` enum mirroring the toolbar entry points
  (`OpenFolder`, `NewNote`, `Save`, `Close`, `ToggleGraph`,
  `ToggleWordWrap`, `ShowPreferences`, `OpenRecent(PathBuf)`);
- an `AppMenu` struct that owns the `muda::Menu`, the File and Recent
  submenus, and a `HashMap<MenuId, MenuAction>` populated at construction;
- `AppMenu::poll()` (drains `MenuEvent::receiver().try_recv()`) and
  `rebuild_recent_submenu(&[PathBuf])` (swaps the submenu in place,
  refreshing the action map so dropped LRU entries stop dispatching).

`muda` requires `NSApplication` to be running, so the menu is initialized
lazily on the first frame of `eframe::App::ui` rather than from
`SideromelaneApp::new`. The shortcut surface matches Spec 0002 AC-3:
⌘O / ⌘N / ⌘S / ⌘W / ⌘G / ⌘⇧W. Preferences gets no accelerator (per spec)
so it doesn't compete with platform conventions like ⌘, that we may want
later for a native-shaped settings hand-off.

For non-macOS targets we ship a feature-gated stub `AppMenu` whose methods
are no-ops. This keeps `cargo check` green on Linux/Windows targets without
dragging the GTK/libxdo deps in for a build that will not ship.

## Consequences

- Native ⌘-key shortcuts work even when `TextEdit` has focus, which fixes
  the long-standing in-window-shortcut frustration.
- New runtime dep: `muda` (plus its small transitive set —
  `keyboard-types`, `crossbeam-channel`, etc.). Licenses already permitted
  by `deny.toml`; no new license waivers needed.
- Recent Folders is rebuilt wholesale on every LRU change. Cheap (cap 10)
  and avoids the alternative of mutating per-entry text via
  `MenuItem::set_text`, which would leave stale `MenuId`-to-path mappings.
- The toolbar buttons stay alongside the menu so neither surface owns the
  feature exclusively — discoverability for new users + native conventions
  for keyboard users.
- Manual smoke testing of menu UX is required before each release; menu
  events are not exercisable from a headless test harness.
