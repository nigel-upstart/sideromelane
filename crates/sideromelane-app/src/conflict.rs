//! Conflict-detection helpers extracted from `main.rs`. The watcher event
//! classification, the canonical-path resolver, and the reload-application
//! helper all live here so the dispatch decisions can be unit-tested without
//! constructing a full `SideromelaneApp`.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use sideromelane_core::NoteId;

use crate::NoteRecord;
use crate::watcher;

/// Maximum number of concurrent pending conflict modals. A large external
/// burst (e.g. `git pull` rewriting many dirty notes at once) would otherwise
/// spawn one `egui::Window` per note. Above the cap we record the overflow
/// count and surface it as a single status message in `render_conflict_modals`
/// so the user knows there are more conflicts queued behind the open ones.
pub const MAX_PENDING_CONFLICTS: usize = 32;

/// Outcome of classifying a single watcher event against the in-memory
/// note set. The caller mutates `folder.notes` / `pending_conflicts` /
/// `self.status` based on this verdict — splitting the decision from the
/// mutation keeps the dispatch testable without standing up a full app.
#[derive(Debug, PartialEq, Eq)]
pub enum WatchOutcome {
    /// Event kind is not modify-class. No-op.
    Ignored,
    /// Event arrived inside the self-write suppression window. No-op.
    Suppressed,
    /// Event path does not match any note in the current folder. No-op.
    UnknownPath,
    /// Note is dirty; queue a per-note conflict modal.
    Conflict(NoteId),
    /// Note is clean; reload `notes[index].source` from disk.
    Reload {
        /// Index into `notes` of the note that should be reloaded.
        index: usize,
    },
}

/// Resolve `path` to its canonical form, falling back to the original on
/// failure. Used by the self-write suppression map so its keys line up with
/// the canonical paths the file watcher delivers (e.g. macOS resolves
/// `/var/folders/...` to `/private/var/folders/...`). When canonicalisation
/// fails (file just deleted, permission denied, …) we keep the original
/// path: a non-canonical match is still valid for paths under non-symlinked
/// roots and is strictly safer than dropping the entry.
pub fn canonicalize_path(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Apply a watcher-driven reload to `notes[index]`. The fresh `source` came
/// straight from disk, so the in-memory copy is now clean again and any
/// subsequent auto-save debounce should be governed by the user's *own* next
/// edit — not by the moment we observed the external write. We therefore
/// clear `dirty` and deliberately leave `last_edit_at` untouched.
pub fn apply_reload(notes: &mut [NoteRecord], index: usize, source: String) {
    let note = &mut notes[index];
    note.source = source;
    note.dirty = false;
}

/// Pure dispatch helper for [`crate::SideromelaneApp::apply_watch_event`]. Splits the
/// suppress / reload / conflict decision from the mutation so it can be unit-
/// tested without constructing a full [`crate::SideromelaneApp`].
pub fn classify_watch_event(
    event: &watcher::WatchEvent,
    notes: &[NoteRecord],
    note_path_index: &HashMap<PathBuf, usize>,
    last_self_write_at: &HashMap<PathBuf, Instant>,
    suppress_window: Duration,
    now: Instant,
) -> WatchOutcome {
    if event.kind != watcher::WatchKind::Modify {
        return WatchOutcome::Ignored;
    }
    if let Some(stamp) = last_self_write_at.get(&event.path)
        && now
            .checked_duration_since(*stamp)
            .is_some_and(|elapsed| elapsed < suppress_window)
    {
        return WatchOutcome::Suppressed;
    }
    // O(1) lookup against the precomputed index. The file-name fallback we
    // used to have was a path-spoof vector (a hostile sibling
    // `attacker/Note.md` could impersonate `notes/Note.md`), so we match
    // strictly by absolute path now. macOS canonicalization (`/private/...`)
    // is reconciled by canonicalising both the watcher event path and the
    // self-write suppression keys (`canonicalize_path`).
    let target_index = note_path_index.get(&event.path).copied();
    match target_index {
        None => WatchOutcome::UnknownPath,
        Some(index) if notes[index].dirty => WatchOutcome::Conflict(notes[index].note_id.clone()),
        Some(index) => WatchOutcome::Reload { index },
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
mod tests {
    use std::collections::HashMap;
    use std::fs;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use sideromelane_core::NoteId;
    use tempfile::TempDir;

    use super::{WatchOutcome, apply_reload, classify_watch_event};
    use crate::NoteRecord;
    use crate::watcher;

    fn note_record(absolute_path: PathBuf, source: &str, last_edit_at: Instant) -> NoteRecord {
        let parent = absolute_path
            .parent()
            .expect("absolute path has parent")
            .to_path_buf();
        let relative = absolute_path
            .strip_prefix(&parent)
            .expect("strip prefix")
            .to_path_buf();
        let note_id = NoteId::from_folder_relative_path(relative).expect("note id");
        NoteRecord {
            note_id,
            absolute_path,
            source: source.to_owned(),
            dirty: true,
            last_edit_at,
        }
    }

    fn watch_event(path: PathBuf, kind: watcher::WatchKind) -> watcher::WatchEvent {
        watcher::WatchEvent { path, kind }
    }

    fn build_index(notes: &[NoteRecord]) -> HashMap<PathBuf, usize> {
        notes
            .iter()
            .enumerate()
            .map(|(idx, note)| (note.absolute_path.clone(), idx))
            .collect()
    }

    #[test]
    fn watch_clean_note_classified_as_reload() {
        let directory = TempDir::new().expect("tempdir");
        let path = directory.path().join("Clean.md");
        fs::write(&path, "x").expect("seed");
        let mut record = note_record(path.clone(), "in memory", Instant::now());
        record.dirty = false;
        let notes = vec![record];
        let index = build_index(&notes);

        let outcome = classify_watch_event(
            &watch_event(path, watcher::WatchKind::Modify),
            &notes,
            &index,
            &HashMap::new(),
            Duration::from_millis(200),
            Instant::now(),
        );
        assert_eq!(outcome, WatchOutcome::Reload { index: 0 });
    }

    #[test]
    fn watch_dirty_note_classified_as_conflict() {
        let directory = TempDir::new().expect("tempdir");
        let path = directory.path().join("Dirty.md");
        fs::write(&path, "x").expect("seed");
        let record = note_record(path.clone(), "in memory", Instant::now());
        let expected = record.note_id.clone();
        let notes = vec![record];
        let index = build_index(&notes);

        let outcome = classify_watch_event(
            &watch_event(path, watcher::WatchKind::Modify),
            &notes,
            &index,
            &HashMap::new(),
            Duration::from_millis(200),
            Instant::now(),
        );
        assert_eq!(outcome, WatchOutcome::Conflict(expected));
    }

    #[test]
    fn watch_event_within_self_write_window_suppressed() {
        let directory = TempDir::new().expect("tempdir");
        let path = directory.path().join("Recent.md");
        fs::write(&path, "x").expect("seed");
        let mut record = note_record(path.clone(), "in memory", Instant::now());
        record.dirty = false;
        let notes = vec![record];
        let index = build_index(&notes);

        let now = Instant::now();
        let mut self_writes = HashMap::new();
        self_writes.insert(
            path.clone(),
            now.checked_sub(Duration::from_millis(50))
                .expect("instant has 50ms history"),
        );

        let outcome = classify_watch_event(
            &watch_event(path, watcher::WatchKind::Modify),
            &notes,
            &index,
            &self_writes,
            Duration::from_millis(200),
            now,
        );
        assert_eq!(outcome, WatchOutcome::Suppressed);
    }

    #[test]
    fn watch_event_for_unknown_path_classified_as_unknown() {
        let directory = TempDir::new().expect("tempdir");
        let known_path = directory.path().join("Known.md");
        let unknown_path = directory.path().join("Unrelated.md");
        fs::write(&known_path, "x").expect("seed");
        let notes = vec![note_record(known_path, "in memory", Instant::now())];
        let index = build_index(&notes);

        let outcome = classify_watch_event(
            &watch_event(unknown_path, watcher::WatchKind::Modify),
            &notes,
            &index,
            &HashMap::new(),
            Duration::from_millis(200),
            Instant::now(),
        );
        assert_eq!(outcome, WatchOutcome::UnknownPath);
    }

    #[test]
    fn watch_other_kind_event_classified_as_ignored() {
        let directory = TempDir::new().expect("tempdir");
        let path = directory.path().join("Note.md");
        fs::write(&path, "x").expect("seed");
        let notes = vec![note_record(path.clone(), "in memory", Instant::now())];
        let index = build_index(&notes);

        let outcome = classify_watch_event(
            &watch_event(path, watcher::WatchKind::Other),
            &notes,
            &index,
            &HashMap::new(),
            Duration::from_millis(200),
            Instant::now(),
        );
        assert_eq!(outcome, WatchOutcome::Ignored);
    }

    #[test]
    fn note_path_index_lookup_matches_iteration() {
        let directory = TempDir::new().expect("tempdir");
        let path_a = directory.path().join("A.md");
        let path_b = directory.path().join("B.md");
        let path_c = directory.path().join("C.md");
        for path in [&path_a, &path_b, &path_c] {
            fs::write(path, "x").expect("seed");
        }
        let mut notes = vec![
            note_record(path_a.clone(), "a", Instant::now()),
            note_record(path_b.clone(), "b", Instant::now()),
            note_record(path_c.clone(), "c", Instant::now()),
        ];
        for note in &mut notes {
            note.dirty = false;
        }
        let index = build_index(&notes);

        for (expected_idx, path) in [&path_a, &path_b, &path_c].iter().enumerate() {
            let scanned = notes.iter().position(|note| &&note.absolute_path == path);
            assert_eq!(scanned, Some(expected_idx));

            let outcome = classify_watch_event(
                &watch_event((*path).clone(), watcher::WatchKind::Modify),
                &notes,
                &index,
                &HashMap::new(),
                Duration::from_millis(200),
                Instant::now(),
            );
            assert_eq!(
                outcome,
                WatchOutcome::Reload {
                    index: expected_idx
                }
            );
        }
    }

    #[test]
    fn apply_reload_clears_dirty_and_preserves_last_edit_at() {
        let directory = TempDir::new().expect("tempdir");
        let path = directory.path().join("Reload.md");
        fs::write(&path, "stale\n").expect("seed");

        let last_edit_at = Instant::now();
        let mut record = note_record(path, "stale\n", last_edit_at);
        record.dirty = true;
        let mut notes = vec![record];

        apply_reload(&mut notes, 0, "fresh\n".to_owned());

        assert_eq!(notes[0].source, "fresh\n");
        assert!(!notes[0].dirty, "dirty should be cleared after reload");
        assert_eq!(
            notes[0].last_edit_at, last_edit_at,
            "last_edit_at must not be bumped by an external reload"
        );
    }
}
