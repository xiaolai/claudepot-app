//! Resolve the CURRENT transcript a live CC process is writing to,
//! working around a gap in CC's PID registry.
//!
//! ## The gap
//!
//! CC registers a PID file at `~/.claude/sessions/<pid>.json` that
//! includes the live sessionId, and updates that field through
//! `onSessionSwitch` whenever `switchSession()` runs (`--continue`,
//! `/resume`, etc.). However `/clear` takes the
//! `regenerateSessionId()` path, which mutates `STATE.sessionId`
//! directly and never fires `sessionSwitched.emit(...)`. See CC
//! source:
//!
//!   - `bootstrap/state.ts::regenerateSessionId`
//!   - `commands/clear/conversation.ts:203` (calls regenerate)
//!   - `utils/concurrentSessions.ts:98-103` (the hook that never fires)
//!
//! Result: after any `/clear`, the PID file still advertises the
//! stale sessionId. Consumers that trust it (tailing `<sid>.jsonl`)
//! end up glued to a transcript whose last line is a terminal
//! `end_turn`, while the real session advances in a sibling
//! `<new-sid>.jsonl` they never notice.
//!
//! ## The workaround
//!
//! For each live PID, scan `<projects_dir>/<slug(cwd)>/*.jsonl` and
//! pick the one that was most-recently written AND whose first event
//! timestamp falls within the PID's lifetime. The PID-declared
//! transcript is always a candidate regardless of first-line timestamp
//! — that covers `/resume` into an older session (where the PID file
//! IS correct, but the transcript's first line pre-dates startedAt).
//!
//! ## Assumptions
//!
//! - One live CC process per cwd in the common case. When two live
//!   PIDs share a cwd, we apply a tie-breaker (PID-file sessionId
//!   match, then newer startedAt) — but the caller should consider
//!   the result advisory rather than authoritative.
//! - Subagent transcripts live in `<slug>/<parent-sid>/subagents/`
//!   subdirectories, NOT as top-level siblings, so a plain readdir
//!   won't pick them up (verified against CC 2.1.116 on-disk layout
//!   2026-04-21).
//! - Transcript first lines are immutable once written — CC writes
//!   them in an atomic append and never rewinds. So we cache first-
//!   line timestamps per path and never need to invalidate on growth.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::project_sanitize::sanitize_path;
use crate::session_live::types::PidRecord;

/// Clock-skew tolerance when comparing a transcript's first-event
/// timestamp against the PID's `startedAt`. CC writes the first
/// event tens of milliseconds after process start, and wall clocks
/// can jitter, so we relax the lower bound by 5 s.
const STARTED_AT_SLACK_MS: i64 = 5_000;

/// One candidate transcript discovered during a slug-dir scan.
///
/// `first_event_ts_ms` is retained for its role in the Debug output
/// that shows up in `tracing` diagnostics — `pick_best` itself only
/// needs mtime and the filename. Drop it and the "why was this
/// candidate chosen" investigation gets harder.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct Candidate {
    session_id: String,
    path: PathBuf,
    mtime: SystemTime,
    first_event_ts_ms: Option<i64>,
}

/// Resolver state — holds a cache of first-line timestamps so
/// repeated ticks don't re-open transcripts we've already inspected.
///
/// Thread-safety: not `Sync`. Callers wrap in `tokio::sync::Mutex`
/// (see `LiveRuntime`).
pub struct TranscriptResolver {
    /// `None` means we tried and found no timestamp (unparseable first
    /// line, empty file). Persisted so we don't re-try every tick.
    /// On an evidently mid-write first line, the entry is absent and
    /// we try again next tick.
    first_event_ts: HashMap<PathBuf, Option<i64>>,
}

impl Default for TranscriptResolver {
    fn default() -> Self {
        Self::new()
    }
}

impl TranscriptResolver {
    pub fn new() -> Self {
        Self {
            first_event_ts: HashMap::new(),
        }
    }

    /// Resolve the current transcript's sessionId for a live PID.
    ///
    /// Returns `Some(session_id)` on success, `None` when neither the
    /// PID-declared transcript nor any timestamp-eligible candidate
    /// exists yet (e.g., brand-new session whose first line hasn't
    /// hit disk). Callers retry on the next tick in that case.
    ///
    /// The returned session_id is ALWAYS a `.jsonl` filename stem in
    /// the slug dir, so callers can recompute the path by joining
    /// `<projects_dir>/<slug(cwd)>/<session_id>.jsonl` without a
    /// separate lookup.
    pub fn resolve(&mut self, rec: &PidRecord, projects_dir: &Path) -> Option<String> {
        let slug = sanitize_path(&rec.cwd);
        let slug_dir = projects_dir.join(&slug);

        let candidates = self.scan_slug_dir(&slug_dir, rec);
        pick_best(&candidates, &rec.session_id).map(|c| c.session_id.clone())
    }

    /// Scan `slug_dir` for `.jsonl` candidates that could belong to
    /// this PID. The PID-declared transcript is always included if
    /// it exists; other transcripts must have a first-line timestamp
    /// at or after `pid.started_at_ms - STARTED_AT_SLACK_MS`.
    fn scan_slug_dir(&mut self, slug_dir: &Path, rec: &PidRecord) -> Vec<Candidate> {
        let Ok(entries) = fs::read_dir(slug_dir) else {
            return Vec::new();
        };

        let declared_filename = format!("{}.jsonl", rec.session_id);
        let floor_ms = rec.started_at_ms - STARTED_AT_SLACK_MS;
        let mut out = Vec::new();

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("jsonl") {
                continue;
            }
            let Some(session_id) = path.file_stem().and_then(|s| s.to_str()).map(str::to_owned)
            else {
                continue;
            };
            let Ok(meta) = entry.metadata() else { continue };
            if !meta.is_file() {
                continue;
            }
            let Ok(mtime) = meta.modified() else { continue };

            let is_declared = path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n == declared_filename);

            // The declared transcript is an unconditional candidate —
            // `/resume <old-sid>` updates the PID file but leaves the
            // transcript's pre-existing (old) first-line timestamp
            // untouched, so it would otherwise fail the floor check.
            if !is_declared {
                // Quick mtime filter — a file untouched since before
                // this PID started can't be the one it's writing to.
                if mtime_before_ms(mtime, floor_ms) {
                    continue;
                }
            }

            let first_event_ts_ms = self.first_event_ts_cached(&path);

            // Non-declared candidate must pass the timestamp floor to
            // prove it was authored DURING this PID's lifetime. If we
            // couldn't read the first line yet (mid-write race),
            // skip — try again next tick.
            if !is_declared {
                match first_event_ts_ms {
                    Some(ts) if ts >= floor_ms => {}
                    _ => continue,
                }
            }

            out.push(Candidate {
                session_id,
                path,
                mtime,
                first_event_ts_ms,
            });
        }

        out
    }

    fn first_event_ts_cached(&mut self, path: &Path) -> Option<i64> {
        if let Some(v) = self.first_event_ts.get(path) {
            return *v;
        }
        let parsed = read_first_event_ts(path).ok().flatten();
        // Only cache a hit — a miss (None) might mean "file mid-write
        // still inside the metadata header", which should be retried
        // next tick rather than poisoned.
        if parsed.is_some() {
            self.first_event_ts.insert(path.to_owned(), parsed);
        }
        parsed
    }

    /// Test-only: peek cache size (for asserting we don't re-read
    /// the same file repeatedly).
    #[cfg(test)]
    pub(crate) fn cache_len(&self) -> usize {
        self.first_event_ts.len()
    }
}

/// Pick the best candidate for this PID.
///
/// Preference order (first match wins):
///   1. Newest mtime overall. Ties broken by:
///      a. Declared sessionId (PID-file match), else
///      b. Lexicographic sessionId for determinism in tests.
///   2. If no candidates at all, fall back to the declared sessionId
///      (caller then has try_attach retry next tick if the file's
///      not there yet).
///
/// Returning `None` is reserved for "no candidates AND no declared
/// transcript" — genuinely nothing to bind to.
fn pick_best<'a>(candidates: &'a [Candidate], declared_sid: &str) -> Option<&'a Candidate> {
    if candidates.is_empty() {
        return None;
    }
    let declared_filename = format!("{declared_sid}.jsonl");

    candidates.iter().max_by(|a, b| {
        // Primary: mtime (newer wins).
        match a.mtime.cmp(&b.mtime) {
            std::cmp::Ordering::Equal => {}
            other => return other,
        }
        // Tie-breaker 1: prefer the declared transcript.
        let a_declared = a
            .path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n == declared_filename);
        let b_declared = b
            .path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n == declared_filename);
        match (a_declared, b_declared) {
            (true, false) => return std::cmp::Ordering::Greater,
            (false, true) => return std::cmp::Ordering::Less,
            _ => {}
        }
        // Tie-breaker 2: sessionId sort (deterministic for tests).
        a.session_id.cmp(&b.session_id)
    })
}

/// Scan the first `MAX_HEADER_LINES` lines of a `.jsonl` and return
/// the first top-level `timestamp` we can parse. CC always writes
/// metadata events (`custom-title`, `agent-name`, `file-history-
/// snapshot`) AHEAD of the first timestamped event — verified on-
/// disk against 2.1.116 — so the first line is NOT a reliable
/// signal. Reading only the first line rejects every fresh post-
/// `/clear` transcript as "no timestamp found" and silently breaks
/// the whole resolution chain.
///
/// Returns `Ok(None)` if:
///   - the file is empty (session just created, nothing flushed)
///   - none of the first `MAX_HEADER_LINES` carry a top-level
///     `timestamp` (very unusual — CC always reaches one within 3-4
///     events; still treated as "don't cache, try again next tick")
const MAX_HEADER_LINES: usize = 20;

fn read_first_event_ts(path: &Path) -> io::Result<Option<i64>> {
    use std::io::{BufRead, BufReader};
    let f = fs::File::open(path)?;
    let r = BufReader::new(f);
    for (i, line) in r.lines().enumerate() {
        if i >= MAX_HEADER_LINES {
            break;
        }
        let line = line?;
        if let Some(ts) = extract_ts_ms(&line) {
            return Ok(Some(ts));
        }
    }
    Ok(None)
}

/// Parse either `"timestamp": "..."` (ISO 8601 with offset/Z) or a
/// top-level numeric `timestamp` / `startedAt` into ms-since-epoch.
fn extract_ts_ms(line: &str) -> Option<i64> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    // Most CC transcript events carry `timestamp`. The first line is
    // usually a metadata event (custom-title / file-history-snapshot
    // / user) — which all stamp `timestamp`. We also probe
    // `startedAt` as a second chance for hand-crafted fixtures.
    for field in ["timestamp", "startedAt"] {
        match v.get(field) {
            Some(serde_json::Value::String(s)) => {
                if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
                    return Some(dt.timestamp_millis());
                }
            }
            Some(serde_json::Value::Number(n)) => {
                if let Some(i) = n.as_i64() {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

fn mtime_before_ms(mtime: SystemTime, floor_ms: i64) -> bool {
    let Ok(d) = mtime.duration_since(SystemTime::UNIX_EPOCH) else {
        return true; // pre-epoch mtime — clearly too old
    };
    (d.as_millis() as i64) < floor_ms
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn mk_pid(session_id: &str, cwd: &str, started_at_ms: i64) -> PidRecord {
        PidRecord {
            pid: 42,
            session_id: session_id.into(),
            cwd: cwd.into(),
            started_at_ms,
            updated_at_ms: None,
            version: None,
            kind: None,
            entrypoint: None,
            name: None,
            status: None,
            waiting_for: None,
        }
    }

    /// Write a single-line JSONL whose first (and only) line carries
    /// the given ISO-8601 timestamp. Returns the full path.
    fn write_transcript(
        projects_dir: &Path,
        cwd: &str,
        sid: &str,
        first_line_iso: &str,
    ) -> PathBuf {
        let slug = sanitize_path(cwd);
        let dir = projects_dir.join(&slug);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{sid}.jsonl"));
        let line = format!(
            r#"{{"type":"custom-title","customTitle":"t","sessionId":"{sid}","timestamp":"{first_line_iso}"}}
"#,
        );
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(line.as_bytes()).unwrap();
        path
    }

    /// Bump mtime to now + `offset_secs` (can be negative). Using
    /// `filetime` keeps the test robust against coarse-grained
    /// default mtime resolution.
    fn touch(path: &Path, offset_secs: i64) {
        let now = std::time::SystemTime::now();
        let target = if offset_secs >= 0 {
            now + std::time::Duration::from_secs(offset_secs as u64)
        } else {
            now - std::time::Duration::from_secs((-offset_secs) as u64)
        };
        filetime::set_file_mtime(path, filetime::FileTime::from_system_time(target)).unwrap();
    }

    #[test]
    fn picks_newest_mtime_when_pid_sessionid_is_stale() {
        let td = TempDir::new().unwrap();
        let projects = td.path().join("projects");
        let cwd = "/Users/me/project";
        // PID started 10 minutes ago.
        let started_at = chrono::Utc::now().timestamp_millis() - 10 * 60 * 1000;

        // Declared (stale) transcript — first event right after startup.
        let declared = write_transcript(&projects, cwd, "stale-sid", &iso_from_offset_secs(-590));
        touch(&declared, -500); // last written 500s ago

        // Fresh transcript from a /clear 2 min ago — still being
        // actively written as of a second ago.
        let fresh = write_transcript(&projects, cwd, "fresh-sid", &iso_from_offset_secs(-120));
        touch(&fresh, -1);

        let mut r = TranscriptResolver::new();
        let rec = mk_pid("stale-sid", cwd, started_at);
        assert_eq!(r.resolve(&rec, &projects).as_deref(), Some("fresh-sid"));
    }

    #[test]
    fn keeps_declared_when_it_is_also_the_most_recent() {
        let td = TempDir::new().unwrap();
        let projects = td.path().join("projects");
        let cwd = "/Users/me/project";
        let started_at = chrono::Utc::now().timestamp_millis() - 5 * 60 * 1000;

        // Declared transcript is being actively written.
        let declared = write_transcript(&projects, cwd, "live-sid", &iso_from_offset_secs(-200));
        touch(&declared, -1);

        // An older /clear-ed sibling is stale.
        let sibling = write_transcript(&projects, cwd, "older-sid", &iso_from_offset_secs(-250));
        touch(&sibling, -200);

        let mut r = TranscriptResolver::new();
        let rec = mk_pid("live-sid", cwd, started_at);
        assert_eq!(r.resolve(&rec, &projects).as_deref(), Some("live-sid"));
    }

    #[test]
    fn resume_case_accepts_declared_even_when_first_ts_is_older_than_started_at() {
        let td = TempDir::new().unwrap();
        let projects = td.path().join("projects");
        let cwd = "/Users/me/project";
        // PID started 2 min ago — user just ran `claude --resume`.
        let started_at = chrono::Utc::now().timestamp_millis() - 2 * 60 * 1000;

        // Resumed transcript — first line is from a week ago. Under the
        // naive "first_ts >= started_at" rule it'd be rejected. The
        // declared-always-admissible rule must keep it.
        let resumed = write_transcript(
            &projects,
            cwd,
            "resumed-sid",
            &iso_from_offset_secs(-7 * 24 * 3600),
        );
        touch(&resumed, -1);

        let mut r = TranscriptResolver::new();
        let rec = mk_pid("resumed-sid", cwd, started_at);
        assert_eq!(r.resolve(&rec, &projects).as_deref(), Some("resumed-sid"));
    }

    #[test]
    fn ignores_ancient_unrelated_transcripts_in_the_slug_dir() {
        let td = TempDir::new().unwrap();
        let projects = td.path().join("projects");
        let cwd = "/Users/me/project";
        let started_at = chrono::Utc::now().timestamp_millis() - 60 * 1000;

        // Ancient transcript (first line a year ago, mtime a year ago).
        let ancient = write_transcript(
            &projects,
            cwd,
            "ancient-sid",
            &iso_from_offset_secs(-365 * 24 * 3600),
        );
        touch(&ancient, -365 * 24 * 3600);

        // Declared transcript — actively being written.
        let declared = write_transcript(&projects, cwd, "current-sid", &iso_from_offset_secs(-30));
        touch(&declared, -1);

        let mut r = TranscriptResolver::new();
        let rec = mk_pid("current-sid", cwd, started_at);
        assert_eq!(r.resolve(&rec, &projects).as_deref(), Some("current-sid"));
    }

    #[test]
    fn empty_slug_dir_returns_none() {
        let td = TempDir::new().unwrap();
        let projects = td.path().join("projects");
        std::fs::create_dir_all(projects.join(sanitize_path("/Users/me/empty"))).unwrap();
        let started_at = chrono::Utc::now().timestamp_millis();
        let rec = mk_pid("sid", "/Users/me/empty", started_at);
        let mut r = TranscriptResolver::new();
        assert_eq!(r.resolve(&rec, &projects), None);
    }

    #[test]
    fn missing_slug_dir_returns_none() {
        let td = TempDir::new().unwrap();
        let projects = td.path().join("projects");
        let rec = mk_pid(
            "sid",
            "/Users/me/never-existed",
            chrono::Utc::now().timestamp_millis(),
        );
        let mut r = TranscriptResolver::new();
        assert_eq!(r.resolve(&rec, &projects), None);
    }

    /// First line was written but isn't parseable (mid-write, partial
    /// flush, or CC wrote a non-JSON metadata header). We skip the
    /// candidate — but we do NOT poison the cache, so the next tick
    /// can retry once the line is flushed.
    #[test]
    fn malformed_first_line_is_skipped_without_cache_poisoning() {
        let td = TempDir::new().unwrap();
        let projects = td.path().join("projects");
        let cwd = "/Users/me/project";
        let started_at = chrono::Utc::now().timestamp_millis() - 60 * 1000;

        let slug = sanitize_path(cwd);
        let dir = projects.join(&slug);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("partial-sid.jsonl");
        std::fs::write(&path, "{not valid json\n").unwrap();
        touch(&path, -1);

        let mut r = TranscriptResolver::new();
        let rec = mk_pid("declared", cwd, started_at);
        // No declared transcript exists either → None.
        assert_eq!(r.resolve(&rec, &projects), None);
        // Cache stayed empty so a repaired first line next tick wins.
        assert_eq!(r.cache_len(), 0);
    }

    #[test]
    fn multiple_clears_in_a_row_pick_the_most_recent() {
        let td = TempDir::new().unwrap();
        let projects = td.path().join("projects");
        let cwd = "/Users/me/project";
        let started_at = chrono::Utc::now().timestamp_millis() - 10 * 60 * 1000;

        // Three generations of a /clear chain:
        // audio-check (declared) → house-cleaning → style-tweak (current)
        let a = write_transcript(&projects, cwd, "audio-check", &iso_from_offset_secs(-580));
        touch(&a, -500);
        let b = write_transcript(
            &projects,
            cwd,
            "house-cleaning",
            &iso_from_offset_secs(-500),
        );
        touch(&b, -300);
        let c = write_transcript(&projects, cwd, "style-tweak", &iso_from_offset_secs(-100));
        touch(&c, -1);

        let mut r = TranscriptResolver::new();
        let rec = mk_pid("audio-check", cwd, started_at);
        assert_eq!(r.resolve(&rec, &projects).as_deref(), Some("style-tweak"));
    }

    #[test]
    fn first_line_cache_is_reused_across_calls() {
        let td = TempDir::new().unwrap();
        let projects = td.path().join("projects");
        let cwd = "/Users/me/project";
        let started_at = chrono::Utc::now().timestamp_millis() - 60 * 1000;

        let fresh = write_transcript(&projects, cwd, "s", &iso_from_offset_secs(-30));
        touch(&fresh, -1);

        let mut r = TranscriptResolver::new();
        let rec = mk_pid("s", cwd, started_at);
        r.resolve(&rec, &projects);
        let after_first = r.cache_len();
        r.resolve(&rec, &projects);
        assert_eq!(
            r.cache_len(),
            after_first,
            "second resolve must reuse the cache without re-reading"
        );
        assert_eq!(after_first, 1, "exactly one entry per observed file");
    }

    #[test]
    fn extract_ts_ms_handles_iso_and_numeric() {
        // 2026-04-21T11:05:50.142Z → chrono::parse_from_rfc3339 →
        // 1_776_769_550_142 ms since UNIX_EPOCH. The literal is
        // computed, not memorized — don't "simplify" it.
        assert_eq!(
            extract_ts_ms(r#"{"timestamp":"2026-04-21T11:05:50.142Z"}"#),
            Some(1_776_769_550_142)
        );
        assert_eq!(
            extract_ts_ms(r#"{"startedAt":1776769550142}"#),
            Some(1_776_769_550_142)
        );
        // Neither field present → None.
        assert_eq!(extract_ts_ms(r#"{"type":"custom-title"}"#), None);
        // Empty / malformed → None.
        assert_eq!(extract_ts_ms("[not json"), None);
        assert_eq!(extract_ts_ms(""), None);
    }

    /// CC writes one or more metadata events (`custom-title`,
    /// `agent-name`, `file-history-snapshot`) BEFORE the first
    /// timestamped event. A resolver that reads only the first line
    /// finds `None`, rejects the candidate as "not owned by this
    /// PID", and leaves us glued to the stale declared sid.
    ///
    /// Observed on-disk against CC 2.1.116 on 2026-04-21: the first
    /// event-with-timestamp was at line 4 (an `attachment` tied to
    /// the SessionStart hook). Header scan must cover enough lines
    /// to reach it in every real transcript.
    #[test]
    fn skips_metadata_header_to_find_first_timestamped_event() {
        let td = TempDir::new().unwrap();
        let projects = td.path().join("projects");
        let cwd = "/Users/me/project";
        let started_at = chrono::Utc::now().timestamp_millis() - 10 * 60 * 1000;

        let slug = sanitize_path(cwd);
        let dir = projects.join(&slug);
        std::fs::create_dir_all(&dir).unwrap();
        let fresh = dir.join("fresh-sid.jsonl");

        // Mimic CC's real on-disk header: title, title, snapshot,
        // then the first event to carry a top-level timestamp.
        let first_event_iso = iso_from_offset_secs(-60);
        let body = format!(
            r#"{{"type":"custom-title","customTitle":"style tweak","sessionId":"fresh-sid"}}
{{"type":"custom-title","customTitle":"style tweak","sessionId":"fresh-sid"}}
{{"type":"file-history-snapshot","messageId":"m","snapshot":{{"trackedFileBackups":{{}}}},"isSnapshotUpdate":false}}
{{"type":"attachment","sessionId":"fresh-sid","timestamp":"{first_event_iso}"}}
"#,
        );
        std::fs::write(&fresh, body).unwrap();
        touch(&fresh, -1);

        // A stale (declared) sibling — first event right after PID
        // startup, last written 500s ago.
        let stale = write_transcript(&projects, cwd, "stale-sid", &iso_from_offset_secs(-599));
        touch(&stale, -500);

        let mut r = TranscriptResolver::new();
        let rec = mk_pid("stale-sid", cwd, started_at);
        assert_eq!(
            r.resolve(&rec, &projects).as_deref(),
            Some("fresh-sid"),
            "real-shape transcript (metadata header + late timestamp) must be accepted"
        );
    }

    /// A mid-write transcript that hasn't reached its first
    /// timestamped line yet (only metadata flushed so far) must
    /// NOT be cached as `None` — the next tick should retry once
    /// CC flushes an event.
    #[test]
    fn header_only_file_is_deferred_without_cache_poisoning() {
        let td = TempDir::new().unwrap();
        let projects = td.path().join("projects");
        let cwd = "/Users/me/project";
        let started_at = chrono::Utc::now().timestamp_millis() - 60 * 1000;

        let slug = sanitize_path(cwd);
        let dir = projects.join(&slug);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("just-starting-sid.jsonl");
        std::fs::write(
            &path,
            r#"{"type":"custom-title","customTitle":"x","sessionId":"just-starting-sid"}
"#,
        )
        .unwrap();
        touch(&path, -1);

        let mut r = TranscriptResolver::new();
        let rec = mk_pid("declared", cwd, started_at);
        assert_eq!(r.resolve(&rec, &projects), None);
        assert_eq!(
            r.cache_len(),
            0,
            "header-only file must not poison the cache — retry next tick"
        );
    }

    /// Subdirectories (subagent scratch, `tool-results/` spill) must
    /// never be treated as transcript candidates — readdir includes
    /// them but the `is_file` guard rules them out.
    #[test]
    fn skips_subdirectories_even_if_they_end_in_jsonl() {
        let td = TempDir::new().unwrap();
        let projects = td.path().join("projects");
        let cwd = "/Users/me/project";
        let slug = sanitize_path(cwd);
        std::fs::create_dir_all(projects.join(&slug).join("weird.jsonl")).unwrap();

        let mut r = TranscriptResolver::new();
        let rec = mk_pid("sid", cwd, chrono::Utc::now().timestamp_millis());
        assert_eq!(r.resolve(&rec, &projects), None);
    }

    // ------------------------------------------------------------
    // Helpers — pure utilities, kept at the bottom so the real
    // tests read in narrative order at the top of the module.
    // ------------------------------------------------------------

    fn iso_from_offset_secs(offset_secs: i64) -> String {
        let t = chrono::Utc::now() + chrono::Duration::seconds(offset_secs);
        t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    }
}
