//! Crash-safe file IO primitives for note persistence.
//!
//! See `docs/adr/0009-safe-note-writes.md` for the rationale.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;

/// Write `source` to `path` atomically.
///
/// The algorithm is:
///
/// 1. Ensure the parent directory exists.
/// 2. Write the new content to a sibling `*.md.tmp` file.
/// 3. `sync_all` the temp file so the bytes are durable on disk.
/// 4. Rename the temp file over `path` (POSIX atomic rename within the same filesystem).
/// 5. Best-effort `sync_all` on the parent directory so the rename is durable.
///
/// Step 5 is best-effort because not every platform supports directory fsync.
/// On those platforms the call is a no-op or returns an error that is ignored.
///
/// # Errors
///
/// Returns the first IO error encountered while creating directories, writing,
/// fsyncing, or renaming. The original file at `path` is never truncated by
/// this function; on error the previous content remains intact.
pub fn safe_write(path: &Path, source: &str) -> io::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }

    // Guard against writing through a symlink, which would allow the rename
    // to escape the folder root (e.g. `Note.md → /etc/hosts`).
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "refusing to overwrite symlinked path",
            ));
        }
        _ => {}
    }

    let temporary_path = path.with_extension("md.tmp");

    {
        let mut file = File::create(&temporary_path)?;
        file.write_all(source.as_bytes())?;
        file.sync_all()?;
    }

    fs::rename(&temporary_path, path)?;

    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        // Best-effort: directory fsync isn't supported everywhere.
        let _ = File::open(parent).and_then(|directory| directory.sync_all());
    }

    Ok(())
}

/// Write `payload` to `path` atomically.
///
/// Sibling to [`safe_write`] for non-Markdown writers (app-local state,
/// preferences, etc.). The same temp-file + `sync_data` + rename + best-effort
/// parent-fsync pattern is used. Differs from `safe_write` in two ways:
///
/// 1. Accepts arbitrary bytes rather than a `&str`.
/// 2. Uses a generic `<filename>.tmp` sibling rather than `.md.tmp`.
///
/// Like [`safe_write`], this function refuses to write through a symlink.
/// Even though callers typically target app-owned data directories, those
/// directories are user-writable and a local process can pre-seed the target
/// path as a symlink before first launch.
///
/// # Errors
///
/// Returns the first IO error encountered while creating directories, writing,
/// fsyncing, or renaming.
pub fn safe_write_bytes(path: &Path, payload: &[u8]) -> io::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }

    // Guard against writing through a symlink. `~/Library/Application Support`
    // is user-writable; another local process can pre-seed e.g. `state.json`
    // as a symlink before first launch.
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "refusing to overwrite symlinked path",
            ));
        }
        _ => {}
    }

    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "safe_write_bytes requires a UTF-8 filename",
            )
        })?;
    let temporary_path = path.with_file_name(format!("{file_name}.tmp"));

    {
        let mut file = File::create(&temporary_path)?;
        file.write_all(payload)?;
        file.sync_data()?;
    }

    fs::rename(&temporary_path, path)?;

    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        let _ = File::open(parent).and_then(|directory| directory.sync_all());
    }

    Ok(())
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::{safe_write, safe_write_bytes};

    #[test]
    fn second_write_replaces_first() {
        let directory = TempDir::new().expect("create temp directory");
        let path = directory.path().join("note.md");

        safe_write(&path, "first").expect("first write");
        safe_write(&path, "second").expect("second write");

        let actual = fs::read_to_string(&path).expect("read file");
        assert_eq!(actual, "second");
    }

    #[test]
    fn orphan_temp_file_does_not_corrupt_subsequent_write() {
        let directory = TempDir::new().expect("create temp directory");
        let path = directory.path().join("note.md");
        let orphan = path.with_extension("md.tmp");
        fs::write(&orphan, "stale orphan").expect("seed orphan temp file");

        safe_write(&path, "fresh content").expect("write succeeds despite orphan");

        let actual = fs::read_to_string(&path).expect("read file");
        assert_eq!(actual, "fresh content");
        assert!(
            !orphan.exists(),
            "rename should consume the temp file rather than leaving it behind",
        );
    }

    #[test]
    fn temp_file_does_not_persist_after_successful_write() {
        let directory = TempDir::new().expect("create temp directory");
        let path = directory.path().join("note.md");

        safe_write(&path, "content").expect("write");

        let temp_path = path.with_extension("md.tmp");
        assert!(
            !temp_path.exists(),
            "temp path should not leak as visible state on success",
        );
    }

    #[test]
    fn creates_parent_directories_when_missing() {
        let directory = TempDir::new().expect("create temp directory");
        let path = directory.path().join("nested/dir/note.md");

        safe_write(&path, "content").expect("write");

        assert_eq!(fs::read_to_string(&path).expect("read file"), "content",);
    }

    #[cfg(unix)]
    #[test]
    fn refuses_write_through_symlink() {
        use std::os::unix::fs::symlink;

        let directory = TempDir::new().expect("create temp directory");
        let target_path = directory.path().join("target.md");
        let note_path = directory.path().join("note.md");

        fs::write(&target_path, "original").expect("write target");
        symlink(&target_path, &note_path).expect("create symlink");

        let result = safe_write(&note_path, "new");
        assert!(result.is_err(), "expected Err but got Ok");
        let error = result.expect_err("safe_write must fail on symlink");
        assert_eq!(
            error.kind(),
            std::io::ErrorKind::InvalidInput,
            "expected InvalidInput error kind",
        );
        assert!(
            error
                .to_string()
                .contains("refusing to overwrite symlinked path"),
            "unexpected error message: {error}",
        );

        // Symlink must still point at target.md.
        let link_meta = fs::symlink_metadata(&note_path).expect("symlink still exists");
        assert!(
            link_meta.file_type().is_symlink(),
            "note.md must remain a symlink"
        );

        // Target content must be unchanged.
        assert_eq!(
            fs::read_to_string(&target_path).expect("read target"),
            "original",
        );
    }

    #[cfg(unix)]
    #[test]
    fn safe_write_bytes_refuses_symlinked_target() {
        use std::os::unix::fs::symlink;

        let directory = TempDir::new().expect("create temp directory");
        let target_path = directory.path().join("real_state.json");
        let state_path = directory.path().join("state.json");

        fs::write(&target_path, b"original bytes").expect("write target");
        symlink(&target_path, &state_path).expect("create symlink");

        let result = safe_write_bytes(&state_path, b"new bytes");
        assert!(result.is_err(), "expected Err but got Ok");
        let error = result.expect_err("safe_write_bytes must fail on symlink");
        assert_eq!(
            error.kind(),
            std::io::ErrorKind::InvalidInput,
            "expected InvalidInput error kind",
        );
        assert!(
            error
                .to_string()
                .contains("refusing to overwrite symlinked path"),
            "unexpected error message: {error}",
        );

        // Symlink must still point at the original target.
        let link_meta = fs::symlink_metadata(&state_path).expect("symlink still exists");
        assert!(
            link_meta.file_type().is_symlink(),
            "state.json must remain a symlink",
        );

        // Target content must be unchanged.
        assert_eq!(
            fs::read(&target_path).expect("read target"),
            b"original bytes",
        );
    }
}
