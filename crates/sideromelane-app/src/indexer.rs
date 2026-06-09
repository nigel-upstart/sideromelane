//! Background indexer worker.
//!
//! See `docs/adr/0008-background-indexer.md` for the architecture overview.
//!
//! The indexer owns a single worker thread that performs note discovery,
//! parsing, and index construction off the UI thread. The worker
//! communicates with the UI thread through `std::sync::mpsc` channels and
//! requests an `egui` repaint after every published event so the UI hydrates
//! without polling.
//!
//! The UI side is purely advisory: search, backlinks, and graph reads run
//! against whatever indexes the app currently holds. They start empty, then
//! become eventually consistent as `IndexUpdated` events arrive.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};

use eframe::egui;
use sideromelane_core::{
    FolderIndex, HybridSearchIndex, MarkdownNote, NoteId, WalkOptions, walk_markdown_paths,
};

/// A single note as discovered by the indexer worker.
#[derive(Debug, Clone)]
pub struct NoteRecord {
    /// Identifier relative to the folder root.
    pub note_id: NoteId,
    /// Absolute path on disk.
    pub absolute_path: PathBuf,
    /// File contents at the time of the read.
    pub source: String,
}

/// Commands accepted by the worker thread.
#[derive(Debug)]
pub enum IndexerCommand {
    /// Re-discover and re-parse every Markdown file under `root`.
    Rescan {
        /// Folder root to scan.
        root: PathBuf,
        /// Walk options sourced from the folder's persisted settings.
        options: WalkOptions,
    },
    /// Replace a single note's source and rebuild indexes.
    NoteChanged {
        /// Identity of the note that changed.
        note_id: NoteId,
        /// New full-source contents of the note.
        source: String,
    },
    /// Stop the worker and exit cleanly.
    Shutdown,
}

/// Events published by the worker thread to the UI.
#[derive(Debug)]
pub enum IndexerEvent {
    /// Initial set of notes discovered for a folder root.
    NotesDiscovered(Vec<NoteRecord>),
    /// Search and folder indexes have been (re)built and are ready to swap in.
    IndexUpdated {
        /// Hybrid keyword + semantic search index.
        search: HybridSearchIndex,
        /// Backlink and graph index.
        folder: FolderIndex,
    },
    /// The walker failed while enumerating notes under the folder root.
    ScanFailed {
        /// Folder root that the failed scan was targeting.
        root: PathBuf,
        /// Human-readable error message suitable for the status bar.
        message: String,
    },
}

/// Handle owned by the UI to talk to the background worker.
#[derive(Debug)]
pub struct Indexer {
    commands: Sender<IndexerCommand>,
    events: Receiver<IndexerEvent>,
    handle: Option<JoinHandle<()>>,
}

impl Indexer {
    /// Spawn a new indexer worker bound to the supplied egui context.
    ///
    /// The context is cloned and used by the worker to wake the UI after
    /// every published event.
    #[must_use]
    pub fn new(context: egui::Context) -> Self {
        let (commands_tx, commands_rx) = mpsc::channel::<IndexerCommand>();
        let (events_tx, events_rx) = mpsc::channel::<IndexerEvent>();

        let handle = thread::Builder::new()
            .name("sideromelane-indexer".to_string())
            .spawn(move || worker_loop(&commands_rx, &events_tx, &context))
            .ok();

        Self {
            commands: commands_tx,
            events: events_rx,
            handle,
        }
    }

    /// Send a command to the worker. Errors are swallowed; if the worker is
    /// gone, the UI proceeds with whatever indexes it already holds and
    /// the parent will spawn a fresh `Indexer` on the next folder open.
    pub fn send(&self, command: IndexerCommand) {
        let _ = self.commands.send(command);
    }

    /// Pop the next event from the worker if any are pending.
    pub fn poll(&self) -> Option<IndexerEvent> {
        self.events.try_recv().ok()
    }
}

impl Drop for Indexer {
    fn drop(&mut self) {
        let _ = self.commands.send(IndexerCommand::Shutdown);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn worker_loop(
    commands: &Receiver<IndexerCommand>,
    events: &Sender<IndexerEvent>,
    context: &egui::Context,
) {
    // Holds the most recently observed note set so single-note edits can
    // rebuild the global indexes without re-reading the folder.
    let mut current_notes: Vec<NoteRecord> = Vec::new();
    // The most recent (root, options) pair seen, so a `NoteChanged` for an
    // unknown note can fall back to a fresh discovery rather than silently
    // losing the update.
    let mut last_walk: Option<(PathBuf, WalkOptions)> = None;

    while let Ok(command) = commands.recv() {
        match command {
            IndexerCommand::Rescan { root, options } => {
                let discovered = match discover_notes(&root, &options) {
                    Ok(records) => records,
                    Err(message) => {
                        if events
                            .send(IndexerEvent::ScanFailed {
                                root: root.clone(),
                                message,
                            })
                            .is_err()
                        {
                            return;
                        }
                        context.request_repaint();
                        continue;
                    }
                };
                current_notes.clone_from(&discovered);
                last_walk = Some((root, options));

                if events
                    .send(IndexerEvent::NotesDiscovered(discovered))
                    .is_err()
                {
                    return;
                }
                context.request_repaint();

                let (search, folder) = build_indexes(&current_notes);
                if events
                    .send(IndexerEvent::IndexUpdated { search, folder })
                    .is_err()
                {
                    return;
                }
                context.request_repaint();
            }
            IndexerCommand::NoteChanged { note_id, source } => {
                let known = current_notes
                    .iter_mut()
                    .find(|record| record.note_id == note_id);

                if let Some(record) = known {
                    record.source = source;
                } else if let Some((root, options)) = last_walk.as_ref() {
                    // Brand-new note that the indexer has not seen yet.
                    // Re-discover so the fresh save shows up in subsequent indexes,
                    // but merge against the existing in-memory set so unsaved
                    // edits to other notes survive the rediscovery.
                    match discover_notes(root, options) {
                        Ok(fresh) => {
                            let mut existing: HashMap<NoteId, NoteRecord> =
                                std::mem::take(&mut current_notes)
                                    .into_iter()
                                    .map(|record| (record.note_id.clone(), record))
                                    .collect();
                            let mut merged: Vec<NoteRecord> = Vec::with_capacity(fresh.len());
                            for fresh_record in fresh {
                                if let Some(mut prior) = existing.remove(&fresh_record.note_id) {
                                    // In-memory edits win for `source`, but pick up any
                                    // path change (e.g. rename) from the fresh discovery.
                                    prior.absolute_path = fresh_record.absolute_path;
                                    merged.push(prior);
                                } else {
                                    merged.push(fresh_record);
                                }
                            }
                            // Anything left in `existing` was not seen on disk and is dropped.
                            current_notes = merged;
                            if let Some(record) = current_notes
                                .iter_mut()
                                .find(|record| record.note_id == note_id)
                            {
                                record.source = source;
                            }
                        }
                        Err(message) => {
                            if events
                                .send(IndexerEvent::ScanFailed {
                                    root: root.clone(),
                                    message,
                                })
                                .is_err()
                            {
                                return;
                            }
                            context.request_repaint();
                            continue;
                        }
                    }
                }

                let (search, folder) = build_indexes(&current_notes);
                if events
                    .send(IndexerEvent::IndexUpdated { search, folder })
                    .is_err()
                {
                    return;
                }
                context.request_repaint();
            }
            IndexerCommand::Shutdown => return,
        }
    }
}

fn discover_notes(root: &Path, options: &WalkOptions) -> Result<Vec<NoteRecord>, String> {
    // `walk_markdown_paths` already returns paths sorted; no need to re-sort.
    let paths = walk_markdown_paths(root, options).map_err(|error| error.to_string())?;
    Ok(paths
        .into_iter()
        .filter_map(|absolute_path| read_note(root, absolute_path).ok())
        .collect())
}

fn read_note(root: &Path, absolute_path: PathBuf) -> io::Result<NoteRecord> {
    let relative_path = absolute_path
        .strip_prefix(root)
        .map_err(io::Error::other)?
        .to_path_buf();
    let note_id = NoteId::from_folder_relative_path(relative_path).map_err(io::Error::other)?;
    let source = fs::read_to_string(&absolute_path)?;

    Ok(NoteRecord {
        note_id,
        absolute_path,
        source,
    })
}

fn build_indexes(notes: &[NoteRecord]) -> (HybridSearchIndex, FolderIndex) {
    let parsed: Vec<MarkdownNote> = notes
        .iter()
        .map(|record| MarkdownNote::parse(record.note_id.clone(), record.source.clone()))
        .collect();
    let search = HybridSearchIndex::from_notes(parsed.clone());
    let folder = FolderIndex::from_notes(parsed);
    (search, folder)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn poll_for<F>(indexer: &Indexer, timeout: Duration, mut predicate: F) -> Option<IndexerEvent>
    where
        F: FnMut(&IndexerEvent) -> bool,
    {
        let start = Instant::now();
        while start.elapsed() < timeout {
            if let Some(event) = indexer.poll() {
                if predicate(&event) {
                    return Some(event);
                }
            } else {
                std::thread::sleep(Duration::from_millis(20));
            }
        }
        None
    }

    #[test]
    fn note_changed_rediscovers_unknown_note_ids() {
        use sideromelane_core::SearchQuery;

        let tempdir = tempfile::tempdir().expect("tempdir");
        let existing_path = tempdir.path().join("Existing.md");
        fs::write(&existing_path, "# existing\n").expect("write existing note");

        let indexer = Indexer::new(egui::Context::default());
        indexer.send(IndexerCommand::Rescan {
            root: tempdir.path().to_path_buf(),
            options: WalkOptions::default(),
        });

        let _initial = poll_for(&indexer, Duration::from_secs(2), |event| {
            matches!(event, IndexerEvent::IndexUpdated { .. })
        })
        .expect("expected an initial IndexUpdated event");

        // Simulate a fresh save of a brand-new note appearing on disk.
        let new_path = tempdir.path().join("New.md");
        fs::write(&new_path, "# new\n").expect("write new note");

        indexer.send(IndexerCommand::NoteChanged {
            note_id: NoteId::from_folder_relative_path("New.md").expect("note id"),
            source: "# new\n".to_string(),
        });

        let event = poll_for(&indexer, Duration::from_secs(2), |event| {
            matches!(event, IndexerEvent::IndexUpdated { .. })
        })
        .expect("expected a follow-up IndexUpdated event");

        match event {
            IndexerEvent::IndexUpdated { search, folder } => {
                let new_id = NoteId::from_folder_relative_path("New.md").expect("note id");
                let has_node = folder
                    .graph()
                    .nodes()
                    .iter()
                    .any(|node| node.as_note() == Some(&new_id));
                assert!(
                    has_node,
                    "FolderIndex graph should include the rediscovered note"
                );

                let hits = search.search(&SearchQuery::text("new"));
                assert!(
                    !hits.is_empty(),
                    "search index should return hits for the new note"
                );
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn rescan_omits_excluded_globs_from_notes_and_search() {
        use sideromelane_core::SearchQuery;

        let tempdir = tempfile::tempdir().expect("tempdir");
        fs::write(tempdir.path().join("Keep.md"), "# keep\nvisible phrase\n")
            .expect("write keep note");
        fs::create_dir_all(tempdir.path().join("node_modules")).expect("mkdir node_modules");
        fs::write(
            tempdir.path().join("node_modules").join("Skip.md"),
            "# skip\nsecret phrase\n",
        )
        .expect("write skipped note");

        let indexer = Indexer::new(egui::Context::default());
        indexer.send(IndexerCommand::Rescan {
            root: tempdir.path().to_path_buf(),
            options: WalkOptions {
                excluded_globs: vec!["node_modules/**".to_string()],
                ..WalkOptions::default()
            },
        });

        let discovered = poll_for(&indexer, Duration::from_secs(2), |event| {
            matches!(event, IndexerEvent::NotesDiscovered(_))
        })
        .expect("expected discovered notes");
        match discovered {
            IndexerEvent::NotesDiscovered(records) => {
                assert_eq!(records.len(), 1);
                assert_eq!(
                    records[0].note_id,
                    NoteId::from_folder_relative_path("Keep.md").expect("note id")
                );
            }
            other => panic!("unexpected event: {other:?}"),
        }

        let index_event = poll_for(&indexer, Duration::from_secs(2), |event| {
            matches!(event, IndexerEvent::IndexUpdated { .. })
        })
        .expect("expected index update");
        match index_event {
            IndexerEvent::IndexUpdated { search, folder } => {
                assert_eq!(search.search(&SearchQuery::text("secret")).len(), 0);
                assert_eq!(search.search(&SearchQuery::text("visible")).len(), 1);
                let skipped_id =
                    NoteId::from_folder_relative_path("node_modules/Skip.md").expect("note id");
                assert!(
                    !folder
                        .graph()
                        .nodes()
                        .iter()
                        .any(|node| node.as_note() == Some(&skipped_id))
                );
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }

    #[test]
    fn drop_joins_worker_thread_without_hanging() {
        let indexer = Indexer::new(egui::Context::default());
        let start = Instant::now();
        drop(indexer);
        assert!(
            start.elapsed() < Duration::from_millis(500),
            "drop should not hang"
        );
    }

    #[test]
    fn rescan_publishes_scan_failed_for_missing_root() {
        let indexer = Indexer::new(egui::Context::default());
        let bad_root = PathBuf::from("/this/path/does/not/exist/sideromelane");
        indexer.send(IndexerCommand::Rescan {
            root: bad_root.clone(),
            options: WalkOptions::default(),
        });

        let event = poll_for(&indexer, Duration::from_secs(2), |event| {
            matches!(event, IndexerEvent::ScanFailed { .. })
        })
        .expect("expected a ScanFailed event within 2 seconds");

        match event {
            IndexerEvent::ScanFailed { root, message } => {
                assert_eq!(root, bad_root);
                assert!(
                    !message.is_empty(),
                    "ScanFailed message should not be empty"
                );
            }
            other => panic!("unexpected event: {other:?}"),
        }
    }
}
