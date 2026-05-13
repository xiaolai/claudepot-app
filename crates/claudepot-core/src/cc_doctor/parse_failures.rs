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
use crate::session_export::redact_secrets;

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
    // Redact the raw pty capture before we persist it. `claude
    // doctor` can — directly or via a future debug-output addition
    // upstream — surface `sk-ant-*` Anthropic tokens or `cdp_pat_*`
    // Claudepot PATs verbatim in its output. Encoding the bytes
    // straight into base64 would put those secrets on disk in a
    // recoverable form. Decode lossy (pty bytes are mostly ASCII +
    // UTF-8 box-drawing), redact, then re-encode.
    let as_text = String::from_utf8_lossy(raw);
    let scrubbed = scrub_pat_tokens(&redact_secrets(&as_text));

    let entry = ParseFailureEntry {
        ts_ms: Utc::now().timestamp_millis(),
        cc_version: snapshot.cc_version.clone(),
        claudepot_version: env!("CARGO_PKG_VERSION").to_string(),
        raw_bytes: snapshot.raw_bytes,
        reason: reason.to_string(),
        raw_b64: base64::engine::general_purpose::STANDARD.encode(scrubbed.as_bytes()),
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

/// Mask `cdp_pat_*` (Claudepot personal access tokens). Same shape
/// as `redact_secrets` for `sk-ant-*` — keep the prefix + last four
/// chars so a reader can disambiguate two redacted entries but
/// can't replay the token.
///
/// Idempotent: an already-masked token (`cdp_pat_***xyz1`) survives
/// re-scrubbing untouched because the `*` run interrupts the
/// otherwise-greedy body match.
fn scrub_pat_tokens(input: &str) -> String {
    let needle = "cdp_pat_";
    if !input.contains(needle) {
        return input.to_string();
    }
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut cursor = 0usize;
    while cursor < bytes.len() {
        match find_substr(bytes, cursor, needle.as_bytes()) {
            Some(start) => {
                out.push_str(&input[cursor..start]);
                let prefix_end = start + needle.len();
                // Already-masked? Format is `cdp_pat_***<last4>`.
                if prefix_end < bytes.len() && bytes[prefix_end] == b'*' {
                    // Walk past the existing mask (asterisks + tail).
                    let mut i = prefix_end;
                    while i < bytes.len() && bytes[i] == b'*' {
                        i += 1;
                    }
                    while i < bytes.len() && is_token_byte(bytes[i]) {
                        i += 1;
                    }
                    out.push_str(&input[start..i]);
                    cursor = i;
                    continue;
                }
                let mut tok_end = prefix_end;
                while tok_end < bytes.len() && is_token_byte(bytes[tok_end]) {
                    tok_end += 1;
                }
                let token = &input[start..tok_end];
                out.push_str(&mask_pat(token));
                cursor = tok_end;
            }
            None => {
                out.push_str(&input[cursor..]);
                break;
            }
        }
    }
    out
}

fn find_substr(haystack: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || from + needle.len() > haystack.len() {
        return None;
    }
    haystack[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|i| from + i)
}

fn is_token_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

fn mask_pat(token: &str) -> String {
    // `cdp_pat_<body>` → keep prefix + ***<last4-of-body>. For very
    // short tokens (<= 4 body chars), keep nothing — still better
    // than leaking the whole thing.
    let prefix_len = "cdp_pat_".len();
    if token.len() <= prefix_len {
        return token.to_string();
    }
    let body = &token[prefix_len..];
    if body.len() <= 4 {
        return format!("{}***", &token[..prefix_len]);
    }
    let tail_start = body.len() - 4;
    format!("{}***{}", &token[..prefix_len], &body[tail_start..])
}

fn append_entry(path: &std::path::Path, entry: &ParseFailureEntry) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let line = serde_json::to_string(entry).map_err(std::io::Error::other)?;

    // Defensive perm-tighten BEFORE write on Unix. Two paths share
    // this helper: (a) creation — `OpenOptionsExt::mode(0o600)`
    // takes care of the new-file case directly; (b) append to an
    // existing file — we chmod to 0o600 first if it isn't already,
    // so an old log from a previous Claudepot build that landed
    // at 0o644 doesn't briefly leak newly-appended sensitive data
    // between `write_all` and the post-hoc chmod.
    #[cfg(unix)]
    tighten_existing_permissions(path)?;

    let mut opts = OpenOptions::new();
    opts.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(path)?;
    f.write_all(line.as_bytes())?;
    f.write_all(b"\n")?;
    Ok(())
}

/// Set `path` to 0o600 if it already exists and is wider. No-op
/// when the file is missing (the create path handles that) or
/// already 0o600. Returns errors from stat/chmod so the caller can
/// log them; the outer `record_parse_failure` swallows them so the
/// JSONL log is best-effort.
#[cfg(unix)]
fn tighten_existing_permissions(path: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };
    let mode = meta.permissions().mode() & 0o777;
    if mode == 0o600 {
        return Ok(());
    }
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
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

    #[test]
    fn scrub_cdp_pat_masks_body_keeping_last_four() {
        let input = "leaked cdp_pat_xtWMoKSEK5Ne-nMeKZwqW4zq1Kiu and carry on";
        let out = scrub_pat_tokens(input);
        assert!(out.contains("cdp_pat_***"), "got: {out}");
        // Keep the last four chars so two redacted entries can still
        // be distinguished by readers; same shape as the sk-ant mask.
        assert!(out.contains("1Kiu"), "got: {out}");
        // Body of the token must be gone.
        assert!(!out.contains("xtWMoKSEK5Ne"), "got: {out}");
    }

    #[test]
    fn scrub_cdp_pat_idempotent_on_already_masked() {
        let already = "cdp_pat_***1Kiu";
        assert_eq!(scrub_pat_tokens(already), already);
    }

    #[test]
    fn record_parse_failure_writes_redacted_anthropic_token() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("failures.jsonl");
        let entry = ParseFailureEntry {
            ts_ms: 0,
            cc_version: None,
            claudepot_version: "0.0.0".into(),
            raw_bytes: 64,
            reason: "test".into(),
            // What the production path encodes: the scrubbed text.
            raw_b64: base64::engine::general_purpose::STANDARD.encode(scrub_pat_tokens(
                &redact_secrets("doctor saw sk-ant-oat01-AbCdEfGhIjKlMnOpQr in env\n"),
            )),
        };
        append_entry(&path, &entry).unwrap();
        let loaded = load_all(&path).unwrap();
        assert_eq!(loaded.len(), 1);
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&loaded[0].raw_b64)
            .unwrap();
        let text = String::from_utf8(decoded).unwrap();
        // sk-ant-* must be masked.
        assert!(
            !text.contains("sk-ant-oat01-AbCdEfGhIjKlMnOpQr"),
            "raw token leaked into JSONL: {text}"
        );
        assert!(text.contains("sk-ant-***"), "mask sentinel missing: {text}");
    }

    #[cfg(unix)]
    #[test]
    fn append_entry_tightens_existing_loose_perms_before_write() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("failures.jsonl");

        // Simulate an old file from a prior Claudepot build that
        // landed at 0o644.
        std::fs::write(&path, b"{\"ts_ms\":1,\"raw_b64\":\"\",\"reason\":\"old\",\"raw_bytes\":0,\"cc_version\":null,\"claudepot_version\":\"0.0.0\"}\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o644,
            "precondition: file is loose"
        );

        // Append a new entry — must observe 0o600 BEFORE write,
        // not chmod after.
        append_entry(
            &path,
            &ParseFailureEntry {
                ts_ms: 2,
                cc_version: None,
                claudepot_version: "0.0.0".into(),
                raw_bytes: 0,
                reason: "fresh".into(),
                raw_b64: String::new(),
            },
        )
        .unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "existing-file path must tighten perms; got {mode:o}"
        );
        // And we did write the new entry.
        let loaded = load_all(&path).unwrap();
        assert!(
            loaded.iter().any(|e| e.reason == "fresh"),
            "new entry not appended"
        );
    }

    #[cfg(unix)]
    #[test]
    fn append_entry_creates_with_0o600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("failures.jsonl");
        append_entry(
            &path,
            &ParseFailureEntry {
                ts_ms: 0,
                cc_version: None,
                claudepot_version: "0.0.0".into(),
                raw_bytes: 0,
                reason: "perm test".into(),
                raw_b64: String::new(),
            },
        )
        .unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "JSONL must be user-only readable; got {mode:o}"
        );
    }
}
