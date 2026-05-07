//! Folder file-system watcher.
//!
//! See `docs/adr/0013-file-watch-and-auto-save.md` for the rationale.
//!
//! Wraps `notify-debouncer-mini` with a small UI-friendly facade. Owns the
//! debouncer and the receiver and exposes a non-blocking `poll` so the UI
//! thread can drain events each frame in the same shape as
//! [`crate::indexer::Indexer`]'s `poll`.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use notify::RecursiveMode;
use notify_debouncer_mini::{
    DebounceEventResult, DebouncedEventKind, Debouncer, new_debouncer, notify::RecommendedWatcher,
};

/// Default debouncer tick window. Short enough to feel responsive in the UI,
/// long enough that a sequence of `safe_write` rename-over-temp operations
/// coalesces into a single event per file.
const DEBOUNCE_WINDOW: Duration = Duration::from_millis(200);

/// Coarse classification of the underlying notify event we care about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WatchKind {
    /// File contents (or metadata) were modified, created, or removed. The
    /// debouncer collapses notify's create/modify/remove triplet into a
    /// single `Any` event, which we surface uniformly.
    Modify,
    /// Reserved for future event categories. Not produced today; kept so the
    /// enum is non-exhaustive in spirit without leaning on the unstable
    /// attribute.
    #[allow(dead_code)]
    Other,
}

/// A single debounced filesystem event addressed at one path.
#[derive(Debug, Clone)]
pub struct WatchEvent {
    /// Absolute path of the file or directory the event refers to.
    pub path: PathBuf,
    /// Coarse event category.
    pub kind: WatchKind,
}

/// Owns a [`Debouncer`] watching a folder root and a receiver fed from the
/// debouncer's callback.
///
/// The debouncer must outlive the receiver — dropping `Watcher` shuts down
/// the underlying watch thread cleanly.
pub struct Watcher {
    // Field order matters: `_debouncer` is dropped after `events`, but neither
    // direction matters semantically — the debouncer's drop joins its worker
    // thread and stops sending into the channel.
    _debouncer: Debouncer<RecommendedWatcher>,
    events: Receiver<WatchEvent>,
}

impl std::fmt::Debug for Watcher {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Watcher")
            .field("debouncer", &"<notify_debouncer_mini::Debouncer>")
            .finish()
    }
}

impl Watcher {
    /// Start watching `root` recursively.
    ///
    /// # Errors
    ///
    /// Returns an [`io::Error`] when the underlying notify watcher fails to
    /// initialize (e.g. permission denied, kernel limit reached).
    pub fn new(root: &Path) -> io::Result<Self> {
        let (events_tx, events_rx) = mpsc::channel::<WatchEvent>();

        let mut debouncer = new_debouncer(DEBOUNCE_WINDOW, move |result: DebounceEventResult| {
            let Ok(events) = result else {
                // Errors surface via notify's own error channel; the mini
                // debouncer collapses them into the result type. Drop them
                // here — the UI will rediscover any state divergence on the
                // next user interaction.
                return;
            };
            for event in events {
                let kind = match event.kind {
                    DebouncedEventKind::Any => WatchKind::Modify,
                    _ => WatchKind::Other,
                };
                // Send is best-effort: if the receiver is gone we are about to
                // be dropped anyway.
                let _ = events_tx.send(WatchEvent {
                    path: event.path,
                    kind,
                });
            }
        })
        .map_err(io::Error::other)?;

        debouncer
            .watcher()
            .watch(root, RecursiveMode::Recursive)
            .map_err(io::Error::other)?;

        Ok(Self {
            _debouncer: debouncer,
            events: events_rx,
        })
    }

    /// Pop the next event if any are pending. Non-blocking.
    pub fn poll(&self) -> Option<WatchEvent> {
        self.events.try_recv().ok()
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
mod tests {
    use std::fs;
    use std::time::{Duration, Instant};

    use tempfile::TempDir;

    use super::{WatchKind, Watcher};

    /// Generous upper bound on debounce delivery. The debouncer ticks every
    /// 200 ms by default; on a busy CI runner the first event can arrive
    /// noticeably later than the configured window.
    const POLL_TIMEOUT: Duration = Duration::from_secs(2);

    fn poll_for_event(watcher: &Watcher, timeout: Duration) -> Option<super::WatchEvent> {
        let start = Instant::now();
        while start.elapsed() < timeout {
            if let Some(event) = watcher.poll() {
                return Some(event);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        None
    }

    #[test]
    fn watcher_emits_event_for_external_write() {
        let directory = TempDir::new().expect("create tempdir");
        let watcher = Watcher::new(directory.path()).expect("start watcher");

        // Give the platform watcher a beat to attach before the write.
        std::thread::sleep(Duration::from_millis(100));

        let target = directory.path().join("note.md");
        fs::write(&target, "# external\n").expect("external write");

        let event = poll_for_event(&watcher, POLL_TIMEOUT)
            .expect("expected a WatchEvent within the timeout");

        // FS-event canonicalization can resolve symlinks (`/private/tmp/...`
        // on macOS) so compare by file name rather than the full path.
        assert_eq!(event.path.file_name(), target.file_name());
        assert_eq!(event.kind, WatchKind::Modify);
    }

    #[test]
    fn dropping_watcher_does_not_hang() {
        let directory = TempDir::new().expect("create tempdir");
        let watcher = Watcher::new(directory.path()).expect("start watcher");
        let start = Instant::now();
        drop(watcher);
        assert!(
            start.elapsed() < Duration::from_secs(2),
            "drop should not hang"
        );
    }
}
