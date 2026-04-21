//! Per-file byte-offset tail reader.
//!
//! Claude Code writes JSONL transcripts via `appendFileSync(path,
//! line + '\n', { mode: 0o600 })` (see `sessionStorage.ts:2572`),
//! which is atomic at line boundary. That means a safe tail reader
//! only has to:
//!
//! 1. Remember the last byte offset it consumed.
//! 2. On poll: read from that offset to EOF, keep only bytes up to
//!    the final `\n`, hand the completed lines to the caller, and
//!    advance the offset past the final newline.
//! 3. Handle rotation (truncation or inode change — file deleted and
//!    recreated) by re-seeding from byte 0 and emitting a
//!    `TailProgress::Rotated` signal so the caller can reset state.
//!
//! The reader is deliberately synchronous; the runtime invokes it
//! under `tokio::task::spawn_blocking` on each FSEvents or polling
//! tick. That keeps the code readable and ports cleanly to WSL /
//! Windows where async file-change notification varies in quality.

use std::fs::{File, Metadata};
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// Stateful tail reader for one JSONL file. One instance per tracked
/// transcript; the runtime owns a `HashMap<file_path, FileTail>`.
#[derive(Debug)]
pub struct FileTail {
    path: PathBuf,
    /// Byte offset we've already consumed up to (exclusive).
    offset: u64,
    /// Inode of the last-seen file, used to detect rotation (the
    /// file was replaced even if its path is unchanged).
    inode: Option<u64>,
    /// Size of the last-seen file, used to detect truncation.
    last_size: Option<u64>,
}

/// Outcome of one `poll` call. `new_lines` contains zero or more
/// fully-terminated JSONL lines WITHOUT their trailing `\n`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TailProgress {
    /// Completed lines read this poll. Order matches file order.
    pub new_lines: Vec<String>,
    /// A rotation was detected before this poll. The caller should
    /// discard any stateful interpretation of earlier lines (the
    /// transcript restarted) before processing `new_lines`.
    pub rotated: bool,
    /// The file at `path` no longer exists. `new_lines` may still be
    /// non-empty if the caller had a fresh tail before deletion, but
    /// subsequent polls will keep returning `missing = true` until
    /// the file reappears.
    pub missing: bool,
}

impl FileTail {
    /// Create a tail positioned at EOF — the caller does NOT want
    /// to replay existing content on first attach. This matches the
    /// M1 seed strategy for *in-flight* sessions: the runtime reads
    /// the last ~64 KB separately to prime status, then opens a
    /// tail that only emits lines appended *after* that point.
    pub fn at_eof(path: impl Into<PathBuf>) -> io::Result<Self> {
        let path = path.into();
        let file = File::open(&path)?;
        let md = file.metadata()?;
        Ok(Self {
            path,
            offset: md.len(),
            inode: Some(md.ino()),
            last_size: Some(md.len()),
        })
    }

    /// Create a tail positioned at byte 0 — will replay the full
    /// current contents on the first poll. Useful for tests and for
    /// sessions that appeared *after* the runtime started.
    pub fn at_start(path: impl Into<PathBuf>) -> io::Result<Self> {
        let path = path.into();
        let md = File::open(&path)?.metadata()?;
        Ok(Self {
            path,
            offset: 0,
            inode: Some(md.ino()),
            last_size: Some(md.len()),
        })
    }

    /// Create a tail for a path that may not exist yet. `poll` will
    /// return `missing = true` until the file appears.
    pub fn pending(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            offset: 0,
            inode: None,
            last_size: None,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Read any new complete lines since the last poll.
    pub fn poll(&mut self) -> io::Result<TailProgress> {
        let file = match File::open(&self.path) {
            Ok(f) => f,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                self.inode = None;
                self.last_size = None;
                self.offset = 0;
                return Ok(TailProgress {
                    new_lines: Vec::new(),
                    rotated: false,
                    missing: true,
                });
            }
            Err(e) => return Err(e),
        };
        let md = file.metadata()?;
        let rotated = self.detect_rotation(&md);
        if rotated {
            self.offset = 0;
            self.inode = Some(md.ino());
            self.last_size = Some(md.len());
        }
        let lines = self.read_from_offset(file, md.len())?;
        self.last_size = Some(md.len());
        self.inode = Some(md.ino());
        Ok(TailProgress {
            new_lines: lines,
            rotated,
            missing: false,
        })
    }

    /// Heuristic: the file rotated if its inode changed, OR its size
    /// shrank below our last-seen offset. The second condition
    /// catches truncation-in-place (`> file` in the shell).
    fn detect_rotation(&self, md: &Metadata) -> bool {
        if let Some(prev_inode) = self.inode {
            if prev_inode != md.ino() {
                return true;
            }
        } else {
            // First sighting of the file — by convention we treat
            // `pending → present` as a non-rotation; the caller
            // observes a fresh tail starting at 0.
            return false;
        }
        if md.len() < self.offset {
            return true;
        }
        false
    }

    fn read_from_offset(&mut self, file: File, file_size: u64) -> io::Result<Vec<String>> {
        if file_size <= self.offset {
            return Ok(Vec::new());
        }
        let mut file = file;
        file.seek(SeekFrom::Start(self.offset))?;
        let mut buf = Vec::with_capacity((file_size - self.offset) as usize);
        file.by_ref().take(file_size - self.offset).read_to_end(&mut buf)?;

        // Only consume up to the LAST '\n'. Anything after is a
        // partial write we must defer to the next poll.
        let last_newline = match buf.iter().rposition(|&b| b == b'\n') {
            Some(pos) => pos,
            None => return Ok(Vec::new()),
        };
        let complete = &buf[..=last_newline];

        let mut lines = Vec::new();
        let reader = BufReader::new(complete);
        for line in reader.lines() {
            match line {
                Ok(s) if s.is_empty() => continue,
                Ok(s) => lines.push(s),
                Err(_) => {
                    // Reading from an in-memory slice should not IO-error;
                    // if it does, log via tracing (not stderr) and stop
                    // at the current offset.
                    break;
                }
            }
        }
        self.offset += (last_newline + 1) as u64;
        Ok(lines)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn tmp() -> TempDir {
        TempDir::new().unwrap()
    }

    fn write_all(path: &Path, text: &str) {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(path)
            .unwrap();
        f.write_all(text.as_bytes()).unwrap();
    }

    fn append(path: &Path, text: &str) {
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(path)
            .unwrap();
        f.write_all(text.as_bytes()).unwrap();
    }

    // ── Basic reads ────────────────────────────────────────────────

    #[test]
    fn at_eof_skips_existing_content() {
        let dir = tmp();
        let path = dir.path().join("s.jsonl");
        write_all(&path, "{\"a\":1}\n{\"b\":2}\n");
        let mut t = FileTail::at_eof(&path).unwrap();
        let p = t.poll().unwrap();
        assert!(p.new_lines.is_empty(), "at_eof should skip prior content");
        assert!(!p.rotated && !p.missing);
    }

    #[test]
    fn at_start_reads_all_existing_then_subsequent_appends() {
        let dir = tmp();
        let path = dir.path().join("s.jsonl");
        write_all(&path, "{\"a\":1}\n{\"b\":2}\n");
        let mut t = FileTail::at_start(&path).unwrap();
        let p = t.poll().unwrap();
        assert_eq!(p.new_lines, vec!["{\"a\":1}", "{\"b\":2}"]);

        append(&path, "{\"c\":3}\n");
        let p = t.poll().unwrap();
        assert_eq!(p.new_lines, vec!["{\"c\":3}"]);
    }

    #[test]
    fn multiple_polls_make_no_duplicate_progress() {
        let dir = tmp();
        let path = dir.path().join("s.jsonl");
        write_all(&path, "{\"a\":1}\n");
        let mut t = FileTail::at_start(&path).unwrap();
        let first = t.poll().unwrap().new_lines;
        let second = t.poll().unwrap().new_lines;
        assert_eq!(first, vec!["{\"a\":1}"]);
        assert!(second.is_empty());
    }

    // ── Partial-line handling ──────────────────────────────────────

    #[test]
    fn partial_line_at_eof_is_deferred() {
        let dir = tmp();
        let path = dir.path().join("s.jsonl");
        write_all(&path, "{\"a\":1}\n{\"b\":");
        let mut t = FileTail::at_start(&path).unwrap();
        let p = t.poll().unwrap();
        // Only the terminated line is surfaced.
        assert_eq!(p.new_lines, vec!["{\"a\":1}"]);
        // Later append completing the partial line + another full line.
        append(&path, "2}\n{\"c\":3}\n");
        let p = t.poll().unwrap();
        assert_eq!(p.new_lines, vec!["{\"b\":2}", "{\"c\":3}"]);
    }

    #[test]
    fn empty_lines_are_skipped() {
        let dir = tmp();
        let path = dir.path().join("s.jsonl");
        write_all(&path, "\n\n{\"a\":1}\n\n");
        let mut t = FileTail::at_start(&path).unwrap();
        let p = t.poll().unwrap();
        assert_eq!(p.new_lines, vec!["{\"a\":1}"]);
    }

    // ── Rotation ───────────────────────────────────────────────────

    #[test]
    fn truncate_in_place_is_detected_as_rotation() {
        let dir = tmp();
        let path = dir.path().join("s.jsonl");
        write_all(&path, "{\"a\":1}\n{\"b\":2}\n");
        let mut t = FileTail::at_start(&path).unwrap();
        let _ = t.poll().unwrap();

        // Truncate the file to a shorter content (simulates `> file`).
        write_all(&path, "{\"x\":1}\n");
        let p = t.poll().unwrap();
        assert!(p.rotated, "truncation must be reported as rotation");
        assert_eq!(p.new_lines, vec!["{\"x\":1}"]);
    }

    #[test]
    fn inode_change_is_detected_as_rotation() {
        let dir = tmp();
        let path = dir.path().join("s.jsonl");
        write_all(&path, "{\"a\":1}\n");
        let mut t = FileTail::at_start(&path).unwrap();
        let _ = t.poll().unwrap();

        // Remove and recreate — new inode.
        std::fs::remove_file(&path).unwrap();
        write_all(&path, "{\"y\":1}\n");
        let p = t.poll().unwrap();
        assert!(p.rotated, "inode change must be reported as rotation");
        assert_eq!(p.new_lines, vec!["{\"y\":1}"]);
    }

    // ── Missing file ───────────────────────────────────────────────

    #[test]
    fn pending_path_returns_missing_until_file_appears() {
        let dir = tmp();
        let path = dir.path().join("s.jsonl");
        let mut t = FileTail::pending(&path);
        let p = t.poll().unwrap();
        assert!(p.missing);
        assert!(p.new_lines.is_empty());

        write_all(&path, "{\"a\":1}\n");
        let p = t.poll().unwrap();
        assert!(!p.missing);
        assert_eq!(p.new_lines, vec!["{\"a\":1}"]);
    }

    #[test]
    fn deletion_after_tailing_reports_missing_without_error() {
        let dir = tmp();
        let path = dir.path().join("s.jsonl");
        write_all(&path, "{\"a\":1}\n");
        let mut t = FileTail::at_start(&path).unwrap();
        let _ = t.poll().unwrap();
        std::fs::remove_file(&path).unwrap();
        let p = t.poll().unwrap();
        assert!(p.missing);
        assert!(p.new_lines.is_empty());
    }

    // ── Large lines (boundary) ─────────────────────────────────────

    #[test]
    fn handles_one_megabyte_line() {
        let dir = tmp();
        let path = dir.path().join("s.jsonl");
        let huge: String = "x".repeat(1_000_000);
        let line = format!(r#"{{"payload":"{huge}"}}"#);
        write_all(&path, &format!("{line}\n"));
        let mut t = FileTail::at_start(&path).unwrap();
        let p = t.poll().unwrap();
        assert_eq!(p.new_lines.len(), 1);
        assert_eq!(p.new_lines[0].len(), line.len());
    }
}
