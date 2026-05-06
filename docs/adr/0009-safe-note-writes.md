# ADR 0009: Safe Note Writes

## Status

Accepted

## Context

The spec requires that app crashes must not corrupt Markdown files and that
saves must avoid truncating the only copy of a note. The previous
`safe_write` wrote to a sibling `*.md.tmp` file and renamed it over the
target, but it did not fsync the temp file or the parent directory. A power
loss or kernel panic between rename and the page cache flush could leave
the target either empty or partially written on disk even though the
rename has logically completed.

## Decision

Save Markdown notes through an atomic write primitive that lives at
`crates/sideromelane-app/src/io.rs` as `safe_write`. The algorithm is:

1. Create the parent directory if it does not exist.
2. Open a sibling temp file at `path.with_extension("md.tmp")`, write the
   full source to it, and call `File::sync_all` on the temp file.
3. `fs::rename` the temp file over the destination (POSIX atomic rename
   within a single filesystem).
4. Best-effort: open the parent directory and call `sync_all` on it so the
   rename itself is durable. Errors are ignored on platforms that do not
   support directory fsync.

A small set of unit tests exercises the happy path, the orphan-temp-file
case, the no-leak invariant, and parent-directory creation.

### Recovery from orphan temp files

If a previous save aborted between `File::create` and `fs::rename`, an
orphan `*.md.tmp` may remain on disk. The next save reuses the same temp
path; the rename overwrites the orphan. v1 does not proactively scan for
or clean up orphans on startup. Doing so would add a first-frame
filesystem walk for a benefit (cosmetic) that does not justify the
startup-budget cost.

### Why no write-ahead log

The unit of user-facing data is a single `.md` file. Per-file atomicity
is sufficient because edits never span files transactionally. A WAL would
add storage, recovery, and supply-chain complexity for a cross-file
guarantee that v1 does not need.

### Module placement

`safe_write` is owned by the app crate today because it is the only
crate that performs note writes. Cohort B's folder-settings module reads
through the same primitive and is expected to depend on it as well. If a
future cohort needs the primitive from `sideromelane-core` (for example
to share with a CLI tool) the function can move into core without
behavioral change.

## Consequences

- A crash mid-save can no longer leave the destination empty or partially
  written. The rename is the single point at which a save becomes
  user-visible.
- Saves now incur two fsyncs (file and best-effort directory). On modern
  SSDs this is well within the typing-latency budget because saves are
  user-initiated rather than per-keystroke.
- The temp-file path is predictable. Tooling that scans the folder
  outside the app can ignore `*.md.tmp` safely.
- Directory fsync is best-effort. On Windows or other platforms without
  directory-level durability the rename is still atomic from the
  application's point of view; only the filesystem-metadata flush is
  weaker.
