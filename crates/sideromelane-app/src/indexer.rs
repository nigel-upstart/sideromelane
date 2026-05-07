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
