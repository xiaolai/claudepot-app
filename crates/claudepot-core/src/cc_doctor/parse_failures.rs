//! Persistent forensic log for `claude doctor` scrape failures.
//!
//! Option A (TUI scraping) trades implementation effort for
//! schema-drift fragility — when CC ships a new doctor layout, our
//! parser produces a `Degraded` or `Failed` snapshot. This module
//! records each failure to disk so the Claudepot developer can read
//! the raw output that broke the parser without having to reproduce
//! the user's environment.
//!
//! Shape:
//!
//! - File: `~/.claudepot/doctor-parse-failures.jsonl` (one entry per
//!   line). JSONL not JSON-array so appending is O(1) without
//!   re-serializing the whole file.
//! - Ring-buffer trim: when the file grows past [`MAX_ENTRIES`] we
//!   rotate by keeping the last N lines. Cheap (single read +
//!   single write, capped file size).
//! - Atomic write via [`crate::fs_utils::atomic_write`] for the
//!   trim path; bare append (file mode 0o600) for the common case.
//! - Best-effort. Every error path logs `tracing::warn!` and
//!   continues — a failed parse-failure write must NOT mask the
//!   original parse failure.
//!
//! Why a separate file from `notifications.json`: notifications are
//! user-facing surface signal; parse failures are developer-facing
//! forensic state. Mixing them would bloat the bell-popover with
//! noise the user can't act on.

use base64::Engine;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;

use crate::cc_doctor::scrape::DoctorSnapshot;

/// Hard cap on retained entries. A failure rate of 1 per day for a
/// year still fits comfortably; the size cap also bounds disk usage
/// since each entry includes the base64-encoded raw bytes (~4 KB
/// expanded for a typical 3 KB doctor capture).
pub const MAX_ENTRIES: usize = 100;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParseFailureEntry {
    /// Wall-clock millis at capture time. Used by the developer
    /// notification to disambiguate "this just happened" from "this
    /// has been broken for a week."
    pub ts_ms: i64,
    /// CC version we parsed out of the snapshot (may be `None` if
    /// the parser couldn't even find that). Critical for diffing
    /// against the last-known-good version.
    pub cc_version: Option<String>,
    /// Claudepot's own version — pinpoints whether the regression
    /// is on our side (parser changed) or theirs (CC updated).
    pub claudepot_version: String,
    /// Total bytes captured. A zero or sub-200 capture is a
    /// different failure shape than a 3 KB capture that didn't
    /// parse — both are recorded but the dev-alert payload calls
    /// out the byte count.
    pub raw_bytes: usize,
    /// Reason string from the [`ParseStatus`] variant.
    pub reason: String,
    /// Base64-encoded raw output. Storing as base64 keeps the JSONL
    /// line valid even when the original bytes are invalid UTF-8 or
    /// carry newlines.
    pub raw_b64: String,
}

/// Default on-disk path. Sibling of `notifications.json` under the
/// same `~/.claudepot/` data directory.
pub fn default_path() -> PathBuf {
    crate::paths::claudepot_data_dir().join("doctor-parse-failures.jsonl")
}

/// Record a single parse failure. Best-effort — every error path
/// logs and continues. Safe to call from any thread.
///
/// `snapshot` carries the parsed metadata (version, install type)
/// to record alongside the raw bytes. `raw` is the pty capture
/// verbatim. `reason` is the human-readable parse status reason.
pub fn record_parse_failure(snapshot: &DoctorSnapshot, raw: &[u8], reason: &str) {
    let entry = ParseFailureEntry {
        ts_ms: Utc::now().timestamp_millis(),
        cc_version: snapshot.cc_version.clone(),
        claudepot_version: env!("CARGO_PKG_VERSION").to_string(),
        raw_bytes: snapshot.raw_bytes,
        reason: reason.to_string(),
        raw_b64: base64::engine::general_purpose::STANDARD.encode(raw),
    };

    let path = default_path();
    if let Err(e) = append_entry(&path, &entry) {
        tracing::warn!("cc_doctor parse-failure log: append failed: {e}");
        return;
    }

    if let Err(e) = maybe_rotate(&path) {
        tracing::warn!("cc_doctor parse-failure log: rotate failed: {e}");
    }

    crate::cc_doctor::dev_alert::dispatch_if_dev_mode(&entry);
}

fn append_entry(path: &std::path::Path, entry: &ParseFailureEntry) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let line = serde_json::to_string(entry).map_err(std::io::Error::other)?;
    let mut f = OpenOptions::new().create(true).append(true).open(path)?;
    f.write_all(line.as_bytes())?;
    f.write_all(b"\n")?;
    // 0o600 on Unix — the file may contain raw pty captures with
    // path strings. The user's home dir is already 0o700 in
    // practice; this is belt-and-suspenders.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

fn maybe_rotate(path: &std::path::Path) -> std::io::Result<()> {
    let contents = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    let lines: Vec<&str> = contents.lines().collect();
    if lines.len() <= MAX_ENTRIES {
        return Ok(());
    }
    let keep = lines[lines.len() - MAX_ENTRIES..].join("\n");
    let mut bytes = keep.into_bytes();
    bytes.push(b'\n');
    crate::fs_utils::atomic_write(path, &bytes)
}

/// Read all recorded failures, newest first. Bounded by the rotation
/// cap. Used by the dev-alert layer to decide "this is a fresh break"
/// vs "the parser has been broken for a week — stop nagging."
pub fn load_all(path: &std::path::Path) -> std::io::Result<Vec<ParseFailureEntry>> {
    let contents = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut out: Vec<ParseFailureEntry> = contents
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    out.reverse();
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cc_doctor::scrape::{DoctorSeverity, ParseStatus};

    fn mk_snapshot() -> DoctorSnapshot {
        DoctorSnapshot {
            cc_version: Some("2.1.138".into()),
            install_type: Some("native".into()),
            install_path: None,
            severity: DoctorSeverity::Warning,
            sections: vec![],
            raw_bytes: 1234,
            parse_status: ParseStatus::Failed {
                reason: "test".into(),
            },
            captured_at_ms: 0,
        }
    }

    #[test]
    fn append_and_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("failures.jsonl");
        let snap = mk_snapshot();

        append_entry(
            &path,
            &ParseFailureEntry {
                ts_ms: 42,
                cc_version: snap.cc_version.clone(),
                claudepot_version: "0.0.0".into(),
                raw_bytes: 0,
                reason: "first".into(),
                raw_b64: String::new(),
            },
        )
        .unwrap();
        append_entry(
            &path,
            &ParseFailureEntry {
                ts_ms: 99,
                cc_version: snap.cc_version.clone(),
                claudepot_version: "0.0.0".into(),
                raw_bytes: 0,
                reason: "second".into(),
                raw_b64: String::new(),
            },
        )
        .unwrap();

        let loaded = load_all(&path).unwrap();
        assert_eq!(loaded.len(), 2);
        // Newest first.
        assert_eq!(loaded[0].ts_ms, 99);
        assert_eq!(loaded[0].reason, "second");
        assert_eq!(loaded[1].ts_ms, 42);
    }

    #[test]
    fn rotate_keeps_last_max_entries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("failures.jsonl");

        for i in 0..(MAX_ENTRIES + 50) {
            append_entry(
                &path,
                &ParseFailureEntry {
                    ts_ms: i as i64,
                    cc_version: None,
                    claudepot_version: "0.0.0".into(),
                    raw_bytes: 0,
                    reason: format!("e{i}"),
                    raw_b64: String::new(),
                },
            )
            .unwrap();
        }
        maybe_rotate(&path).unwrap();

        let loaded = load_all(&path).unwrap();
        assert_eq!(loaded.len(), MAX_ENTRIES);
        // load_all reverses (newest first); index 0 = highest ts.
        assert_eq!(loaded[0].reason, format!("e{}", MAX_ENTRIES + 49));
    }
}
