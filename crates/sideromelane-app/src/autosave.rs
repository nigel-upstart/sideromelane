//! Auto-save sweep helpers extracted from `main.rs`. The sweep itself is
//! synchronous and does no UI work, so it lives here in pure-data form;
//! `SideromelaneApp::auto_save_tick` calls in to drive the indexer/status
//! updates after the borrow on `notes` is released.

use std::io as std_io;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use sideromelane_core::NoteId;

use crate::NoteRecord;
use crate::io::safe_write;

/// Window after a successful self-initiated `safe_write` during which any
/// watcher event for the same path is treated as our own write and ignored.
/// See ADR 0013.
pub const SELF_WRITE_SUPPRESS_WINDOW: Duration = Duration::from_millis(200);

/// One successfully auto-saved note. Owns clones of the fields the caller
/// needs to drive status updates and the indexer rebuild after the borrow on
/// `notes` is released.
#[derive(Debug)]
pub struct AutoSaveOutcome {
    pub note_id: NoteId,
    pub source: String,
    pub absolute_path: PathBuf,
    pub relative: String,
}

/// Aggregated result of one auto-save sweep. `first_error` carries the first
/// `safe_write` failure encountered so the UI can surface it; subsequent
/// errors are not collected because they would all share the same status
/// slot anyway.
#[derive(Debug, Default)]
pub struct AutoSaveSweep {
    pub saved: Vec<AutoSaveOutcome>,
    pub first_error: Option<(String, std_io::Error)>,
}

/// Pure-ish auto-save iteration helper. Walks `notes`, calls `safe_write`
/// on each one whose last edit is older than `debounce`, clears `dirty`
/// on success, and returns the per-note outcomes for the caller to thread
/// through the indexer and status bar without holding a `&mut FolderState`.
///
/// `now` is taken as a parameter so tests can simulate a debounce timeout
/// without `std::thread::sleep`.
pub fn auto_save_dirty_notes(
    notes: &mut [NoteRecord],
    debounce: Duration,
    now: Instant,
) -> AutoSaveSweep {
    let mut sweep = AutoSaveSweep::default();
    for note in notes.iter_mut() {
        if !note.dirty {
            continue;
        }
        if now.saturating_duration_since(note.last_edit_at) < debounce {
            continue;
        }
        match safe_write(&note.absolute_path, &note.source) {
            Ok(()) => {
                note.dirty = false;
                let relative = note.note_id.relative_path().display().to_string();
                sweep.saved.push(AutoSaveOutcome {
                    note_id: note.note_id.clone(),
                    source: note.source.clone(),
                    absolute_path: note.absolute_path.clone(),
                    relative,
                });
            }
            Err(error) => {
                if sweep.first_error.is_none() {
                    let relative = note.note_id.relative_path().display().to_string();
                    sweep.first_error = Some((relative, error));
                }
            }
        }
    }
    sweep
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{Duration, Instant};

    use sideromelane_core::NoteId;
    use tempfile::TempDir;

    use super::auto_save_dirty_notes;
    use crate::NoteRecord;

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

    #[test]
    fn auto_save_tick_writes_dirty_after_debounce() {
        let directory = TempDir::new().expect("tempdir");
        let path = directory.path().join("Note.md");
        fs::write(&path, "stale\n").expect("seed file");

        let now = Instant::now();
        let last_edit_at = now
            .checked_sub(Duration::from_secs(6))
            .expect("instant has six-second history");
        let mut notes = vec![note_record(path.clone(), "fresh body", last_edit_at)];

        let sweep = auto_save_dirty_notes(&mut notes, Duration::from_secs(5), now);

        assert_eq!(sweep.saved.len(), 1);
        assert!(sweep.first_error.is_none());
        assert!(!notes[0].dirty);
        assert_eq!(fs::read_to_string(&path).expect("read note"), "fresh body");
    }

    #[test]
    fn auto_save_tick_skips_recently_edited_notes() {
        let directory = TempDir::new().expect("tempdir");
        let path = directory.path().join("Recent.md");
        fs::write(&path, "previous\n").expect("seed file");

        let now = Instant::now();
        let last_edit_at = now
            .checked_sub(Duration::from_secs(1))
            .expect("instant has one-second history");
        let mut notes = vec![note_record(path.clone(), "in progress", last_edit_at)];

        let sweep = auto_save_dirty_notes(&mut notes, Duration::from_secs(5), now);

        assert!(sweep.saved.is_empty());
        assert!(notes[0].dirty);
        assert_eq!(fs::read_to_string(&path).expect("read note"), "previous\n");
    }

    #[test]
    fn auto_save_tick_ignores_clean_notes() {
        let directory = TempDir::new().expect("tempdir");
        let path = directory.path().join("Clean.md");
        fs::write(&path, "synced\n").expect("seed file");

        let now = Instant::now();
        let last_edit_at = now
            .checked_sub(Duration::from_mins(1))
            .expect("instant has one-minute history");
        let mut record = note_record(path.clone(), "in memory", last_edit_at);
        record.dirty = false;
        let mut notes = vec![record];

        let sweep = auto_save_dirty_notes(&mut notes, Duration::from_secs(5), now);

        assert!(sweep.saved.is_empty());
        assert_eq!(fs::read_to_string(&path).expect("read"), "synced\n");
    }
}
