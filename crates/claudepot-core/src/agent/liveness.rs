//! Is an agent run actually in flight?
//!
//! # Why this is not a boolean in the renderer
//!
//! The Agents pane used to derive "running" from a renderer-local flag
//! set on click and cleared by a five-minute timer. A real run takes
//! ~15 minutes, so the card returned to idle two thirds of the way
//! through and re-enabled `Run Now`; a cron-fired run — the entire
//! reason the `agent` noun exists — set the flag never and was invisible
//! for its whole life. See `dev-docs/agents-run-visibility-plan.md` §1.
//!
//! State is therefore **observed**, from two independent signals:
//!
//! 1. **`result.json` absent** — the run has not recorded an outcome.
//! 2. **`run.pid` alive** — a process with that pid exists.
//!
//! # Why both, and never just the first
//!
//! Signal 1 alone has two causes: the run is alive, or it died hard and
//! never wrote anything. Collapsing them would render a crashed run as
//! "Running" forever — a card reading "Running 3 days". That is the same
//! defect class as reporting a failed scan as "nothing scheduled for
//! deletion", so the unknown gets its own state
//! ([`RunState::Interrupted`]) rather than being folded into either
//! confident answer.
//!
//! Run directories created before the shim wrote `run.pid` have no pid
//! file at all. They resolve to `Interrupted`, which is the safe
//! direction: a stale run reads as "no result recorded", never as live.

use std::path::{Path, PathBuf};

pub use crate::session_live::registry::ProcessCheck;

/// A scan that could not be completed. Deliberately distinct from an
/// empty result: "no runs" and "could not look" are different answers,
/// and only one of them is safe to render as idle.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ScanError {
    #[error("agent run directory is unreadable: {0}")]
    Unreadable(String),
}

/// What a run directory without a recorded result actually is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    /// `result.json` absent and the pid is alive.
    Running,
    /// `result.json` absent and the pid is gone, unreadable, or was
    /// never written. The run produced no outcome and nothing is
    /// working on it — say so rather than guessing which.
    Interrupted,
}

/// One run directory that has not recorded a result.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct InFlightRun {
    pub agent_id: String,
    pub run_id: String,
    /// Directory mtime in epoch ms — when the run started. The UI
    /// computes elapsed from this rather than being told a duration, so
    /// a slow poll never makes the clock stutter.
    pub started_ms: i64,
    pub state: RunState,
}

/// Read `<run_dir>/run.pid`. `None` when absent, unreadable, or not a
/// plausible pid — all of which mean "cannot attribute a process to this
/// run", which is `Interrupted`, not `Running`.
fn read_pid(run_dir: &Path) -> Option<u32> {
    let raw = std::fs::read_to_string(run_dir.join("run.pid")).ok()?;
    let pid: u32 = raw.trim().parse().ok()?;
    (pid > 0).then_some(pid)
}

fn dir_started_ms(run_dir: &Path) -> i64 {
    std::fs::metadata(run_dir)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

/// Scan one agent's `runs/` directory for runs with no recorded result.
///
/// `agent_dir` is `~/.claudepot/agents/<id>`. Pure with respect to the
/// process table: the check is injected, so every state below is unit
/// tested against a fixture tree with no real processes.
pub fn scan_agent_dir(
    agent_id: &str,
    agent_dir: &Path,
    check: &dyn ProcessCheck,
) -> Result<Vec<InFlightRun>, ScanError> {
    let runs_dir: PathBuf = agent_dir.join("runs");
    let rd = match std::fs::read_dir(&runs_dir) {
        Ok(rd) => rd,
        // A MISSING runs directory is a real zero — the agent has never
        // run, and CC creates it lazily. Anything else (permissions, a
        // transient I/O error) is NOT a zero: rendering it as "never
        // ran" is precisely the "failed read is not a clean result"
        // failure this module exists to prevent.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Ok(Vec::new());
        }
        Err(e) => return Err(ScanError::Unreadable(e.to_string())),
    };

    // Collect first so the process check can be primed with every pid in
    // one refresh rather than one system call per run.
    let mut candidates: Vec<(String, PathBuf, Option<u32>)> = Vec::new();
    for ent in rd.flatten() {
        if !ent.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let dir = ent.path();
        if dir.join("result.json").exists() {
            continue; // finished — history's problem, not liveness'
        }
        let Some(run_id) = ent.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let pid = read_pid(&dir);
        candidates.push((run_id, dir, pid));
    }

    let pids: Vec<u32> = candidates.iter().filter_map(|(_, _, p)| *p).collect();
    check.prime(&pids);

    let out = candidates
        .into_iter()
        .map(|(run_id, dir, pid)| InFlightRun {
            agent_id: agent_id.to_string(),
            run_id,
            started_ms: dir_started_ms(&dir),
            state: match pid {
                Some(p) if check.is_running(p) => RunState::Running,
                _ => RunState::Interrupted,
            },
        })
        .collect::<Vec<_>>();
    Ok(out)
}

/// The most recent run that DID record a result.
///
/// Carried by the same poll that reports in-flight runs, rather than
/// bolted onto `AgentSummaryDto`: that DTO is a pure `From<&Agent>`
/// conversion, and giving it filesystem reads would make every list call
/// touch disk for data only this pane wants. One poll, one source of
/// truth for run state.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LastRun {
    pub run_id: String,
    pub ended_ms: i64,
    pub exit_code: i32,
    pub duration_ms: i64,
}

/// Everything the pane needs to describe one agent's run state.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AgentRunStatus {
    pub agent_id: String,
    /// `None` when nothing is in flight.
    pub in_flight: Option<InFlightRun>,
    /// `None` when the agent has never completed a run — which is a
    /// different statement from "the last run failed", and the card
    /// renders them differently.
    pub last: Option<LastRun>,
    /// This agent's run data could not be read (permissions, transient
    /// I/O, an unparseable newest result). Per-agent rather than global
    /// so one bad directory does not blank every other card. When set,
    /// `in_flight` and `last` are NOT trustworthy and the UI renders
    /// "can't determine" rather than idle.
    pub unreadable: bool,
    /// Set only on the synthetic sentinel returned when the agents ROOT
    /// itself is unreadable. Carries the reason so the UI can say what
    /// went wrong instead of rendering every card as idle.
    #[serde(default)]
    pub root_error: Option<String>,
}

/// Newest `result.json` under an agent's `runs/`, if any.
///
/// Reads only the four fields the card needs; a malformed or truncated
/// `result.json` yields `None` rather than a partial record, because a
/// half-parsed outcome rendered as fact is the failure this whole area
/// keeps producing.
pub fn last_run(agent_dir: &Path) -> Result<Option<LastRun>, ScanError> {
    let rd = match std::fs::read_dir(agent_dir.join("runs")) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(ScanError::Unreadable(e.to_string())),
    };

    // Find the NEWEST recorded result first, then parse exactly that
    // one. Parsing every candidate and keeping the newest that happened
    // to parse would let a truncated newest result be silently replaced
    // by an older run — the card would then state an old success or
    // failure as the current fact. An unparseable newest result is an
    // unknown, and unknowns are never rendered as confident answers.
    let mut newest: Option<(i64, String, PathBuf)> = None;
    for ent in rd.flatten() {
        let dir = ent.path();
        let result_path = dir.join("result.json");
        let Ok(meta) = std::fs::metadata(&result_path) else {
            continue; // no result.json — that run is in flight, not last
        };
        let ended_ms = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .and_then(|d| i64::try_from(d.as_millis()).ok())
            .unwrap_or(0);
        let run_id = ent.file_name().to_str().unwrap_or_default().to_string();
        if newest.as_ref().is_none_or(|(t, _, _)| ended_ms > *t) {
            newest = Some((ended_ms, run_id, result_path));
        }
    }

    let Some((ended_ms, run_id, path)) = newest else {
        return Ok(None); // never completed a run
    };

    let text = std::fs::read_to_string(&path).map_err(|e| ScanError::Unreadable(e.to_string()))?;
    let v: serde_json::Value = serde_json::from_str(&text)
        .map_err(|e| ScanError::Unreadable(format!("{}: {e}", path.display())))?;
    let (Some(exit_code), Some(duration_ms)) = (
        v.get("exit_code").and_then(|x| x.as_i64()),
        v.get("duration_ms").and_then(|x| x.as_i64()),
    ) else {
        return Err(ScanError::Unreadable(format!(
            "{}: missing exit_code or duration_ms",
            path.display()
        )));
    };

    Ok(Some(LastRun {
        run_id,
        ended_ms,
        exit_code: i32::try_from(exit_code).unwrap_or(i32::MAX),
        duration_ms,
    }))
}

/// Per-agent run status for every agent in the live data directory.
pub fn scan_status_live(check: &dyn ProcessCheck) -> Vec<AgentRunStatus> {
    let root = crate::paths::claudepot_data_dir().join("agents");
    let rd = match std::fs::read_dir(&root) {
        Ok(rd) => rd,
        // A missing agents root is a real zero: no agent has ever been
        // installed. Anything else is unknown, and returning an empty
        // vec would render every card idle — the same failure as the
        // per-agent case below, one level up.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => {
            return vec![AgentRunStatus {
                agent_id: String::new(),
                in_flight: None,
                last: None,
                unreadable: true,
                root_error: Some(e.to_string()),
            }]
        }
    };
    let mut out = Vec::new();
    for ent in rd.flatten() {
        if !ent.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let Some(id) = ent.file_name().to_str().map(str::to_string) else {
            continue;
        };
        let dir = ent.path();
        let (in_flight, scan_bad) = match scan_agent_dir(&id, &dir, check) {
            Ok(mut v) => {
                // Newest first; the pane shows one.
                v.sort_by(|a, b| b.started_ms.cmp(&a.started_ms));
                (v.into_iter().next(), false)
            }
            Err(_) => (None, true),
        };
        let (last, last_bad) = match last_run(&dir) {
            Ok(l) => (l, false),
            Err(_) => (None, true),
        };
        out.push(AgentRunStatus {
            agent_id: id,
            in_flight,
            last,
            unreadable: scan_bad || last_bad,
            root_error: None,
        });
    }
    out
}

/// Scan every agent under `agents_root` (`~/.claudepot/agents`).
pub fn scan_all(agents_root: &Path, check: &dyn ProcessCheck) -> Vec<InFlightRun> {
    let Ok(rd) = std::fs::read_dir(agents_root) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for ent in rd.flatten() {
        if !ent.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let Some(id) = ent.file_name().to_str().map(str::to_string) else {
            continue;
        };
        out.extend(scan_agent_dir(&id, &ent.path(), check).unwrap_or_default());
    }
    // Newest first, so a UI that shows only one shows the current run.
    out.sort_by(|a, b| b.started_ms.cmp(&a.started_ms));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::fs;
    use tempfile::TempDir;

    /// Fake table: only the pids handed in are "alive".
    struct Fake(HashSet<u32>);
    impl ProcessCheck for Fake {
        fn is_running(&self, pid: u32) -> bool {
            self.0.contains(&pid)
        }
    }
    fn alive(pids: &[u32]) -> Fake {
        Fake(pids.iter().copied().collect())
    }

    fn mk_run(agent_dir: &Path, run_id: &str, pid: Option<&str>, result: bool) -> PathBuf {
        let d = agent_dir.join("runs").join(run_id);
        fs::create_dir_all(&d).unwrap();
        if let Some(p) = pid {
            fs::write(d.join("run.pid"), p).unwrap();
        }
        if result {
            fs::write(d.join("result.json"), "{}").unwrap();
        }
        d
    }

    #[test]
    fn a_live_pid_with_no_result_is_running() {
        let t = TempDir::new().unwrap();
        mk_run(t.path(), "r1", Some("4242"), false);
        let got = scan_agent_dir("a", t.path(), &alive(&[4242])).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].state, RunState::Running);
        assert_eq!(got[0].run_id, "r1");
    }

    /// The load-bearing case. A run that died without recording must not
    /// render as live — that is what produces "Running 3 days".
    #[test]
    fn a_dead_pid_with_no_result_is_interrupted_not_running() {
        let t = TempDir::new().unwrap();
        mk_run(t.path(), "r1", Some("4242"), false);
        let got = scan_agent_dir("a", t.path(), &alive(&[])).unwrap(); // nothing alive
        assert_eq!(got[0].state, RunState::Interrupted);
    }

    /// Backward compatibility: directories created before the shim wrote
    /// `run.pid`. They must resolve to Interrupted, never Running.
    #[test]
    fn a_run_predating_the_pid_file_is_interrupted() {
        let t = TempDir::new().unwrap();
        mk_run(t.path(), "old", None, false);
        let got = scan_agent_dir("a", t.path(), &alive(&[4242])).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].state, RunState::Interrupted);
    }

    #[test]
    fn a_finished_run_is_not_in_flight_at_all() {
        let t = TempDir::new().unwrap();
        mk_run(t.path(), "done", Some("4242"), true);
        assert!(scan_agent_dir("a", t.path(), &alive(&[4242]))
            .unwrap()
            .is_empty());
    }

    /// An unparseable or absurd pid is "cannot attribute a process",
    /// which is Interrupted — not a panic and not an optimistic Running.
    #[test]
    fn a_garbage_pid_file_is_interrupted() {
        let t = TempDir::new().unwrap();
        mk_run(t.path(), "bad", Some("not-a-pid"), false);
        mk_run(t.path(), "zero", Some("0"), false);
        let got = scan_agent_dir("a", t.path(), &alive(&[4242])).unwrap();
        assert_eq!(got.len(), 2);
        assert!(got.iter().all(|r| r.state == RunState::Interrupted));
    }

    #[test]
    fn an_agent_that_never_ran_reports_nothing() {
        let t = TempDir::new().unwrap();
        assert!(scan_agent_dir("a", t.path(), &alive(&[]))
            .unwrap()
            .is_empty());
    }

    #[test]
    fn last_run_picks_the_newest_recorded_result() {
        let t = TempDir::new().unwrap();
        let older = mk_run(t.path(), "old", Some("1"), true);
        fs::write(
            older.join("result.json"),
            r#"{"exit_code":1,"duration_ms":100}"#,
        )
        .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        let newer = mk_run(t.path(), "new", Some("2"), true);
        fs::write(
            newer.join("result.json"),
            r#"{"exit_code":0,"duration_ms":886422}"#,
        )
        .unwrap();

        let got = last_run(t.path()).unwrap().expect("a completed run exists");
        assert_eq!(got.run_id, "new");
        assert_eq!(got.exit_code, 0);
        assert_eq!(got.duration_ms, 886422);
    }

    /// "Never completed a run" and "the last run failed" are different
    /// statements and the card renders them differently, so the absent
    /// case must be `None` rather than a zero-valued record.
    #[test]
    fn an_agent_with_no_completed_run_has_no_last_run() {
        let t = TempDir::new().unwrap();
        mk_run(t.path(), "live", Some("1"), false); // in flight, no result
        assert!(last_run(t.path()).unwrap().is_none());
    }

    /// A truncated or malformed result must yield nothing, not a
    /// partially-populated record rendered as fact.
    #[test]
    fn a_malformed_result_json_is_skipped_not_half_parsed() {
        let t = TempDir::new().unwrap();
        let d = mk_run(t.path(), "bad", Some("1"), true);
        fs::write(d.join("result.json"), "{\"exit_code\":").unwrap();
        assert!(
            last_run(t.path()).is_err(),
            "a truncated newest result is UNKNOWN, not absent"
        );

        let t2 = TempDir::new().unwrap();
        let d2 = mk_run(t2.path(), "partial", Some("2"), true);
        // Valid JSON, but missing duration_ms.
        fs::write(d2.join("result.json"), r#"{"exit_code":0}"#).unwrap();
        assert!(last_run(t2.path()).is_err());
    }

    /// #2 from the round-1 audit. A directory that exists but cannot be
    /// read is NOT "the agent never ran" — rendering it as idle is the
    /// exact "failed read is a clean result" failure this module was
    /// written to prevent. Only `NotFound` is a real zero.
    #[cfg(unix)]
    #[test]
    fn an_unreadable_runs_directory_is_an_error_not_an_empty_list() {
        use std::os::unix::fs::PermissionsExt;
        let t = TempDir::new().unwrap();
        let runs = t.path().join("runs");
        fs::create_dir_all(&runs).unwrap();
        fs::set_permissions(&runs, fs::Permissions::from_mode(0o000)).unwrap();

        let got = scan_agent_dir("a", t.path(), &alive(&[]));
        // Restore before asserting so a failure cannot leave a
        // permission-denied directory behind in the temp tree.
        fs::set_permissions(&runs, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(got.is_err(), "unreadable must not read as an empty scan");
    }

    #[test]
    fn a_missing_runs_directory_is_a_real_zero_not_an_error() {
        let t = TempDir::new().unwrap();
        assert_eq!(scan_agent_dir("a", t.path(), &alive(&[])).unwrap(), vec![]);
        assert_eq!(last_run(t.path()).unwrap(), None);
    }

    /// #3 from the round-1 audit, and the sharpest of them. Parsing every
    /// candidate and keeping the newest that PARSED let a truncated
    /// newest result be silently replaced by an older run — so the card
    /// would state a stale success as the current fact.
    #[test]
    fn a_malformed_newest_result_never_falls_back_to_an_older_run() {
        let t = TempDir::new().unwrap();
        let older = mk_run(t.path(), "older", Some("1"), true);
        fs::write(
            older.join("result.json"),
            r#"{"exit_code":0,"duration_ms":100}"#,
        )
        .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        let newest = mk_run(t.path(), "newest", Some("2"), true);
        fs::write(newest.join("result.json"), "{truncated").unwrap();

        let got = last_run(t.path());
        assert!(
            got.is_err(),
            "expected UNKNOWN, got {got:?} — an older run must never be \
             promoted to 'last' because the newest one would not parse"
        );
    }

    #[test]
    fn scan_all_walks_every_agent_and_sorts_newest_first() {
        let t = TempDir::new().unwrap();
        let a = t.path().join("agent-a");
        let b = t.path().join("agent-b");
        mk_run(&a, "r1", Some("1"), false);
        mk_run(&b, "r2", Some("2"), false);
        let got = scan_all(t.path(), &alive(&[1, 2]));
        assert_eq!(got.len(), 2);
        assert!(got[0].started_ms >= got[1].started_ms, "newest first");
        let ids: HashSet<&str> = got.iter().map(|r| r.agent_id.as_str()).collect();
        assert!(ids.contains("agent-a") && ids.contains("agent-b"));
    }

    /// The process table is probed once per scan, not once per run —
    /// `prime` receives every pid up front.
    #[test]
    fn the_process_check_is_primed_with_every_pid_at_once() {
        use std::sync::Mutex;
        struct Recording(Mutex<Vec<Vec<u32>>>);
        impl ProcessCheck for Recording {
            fn is_running(&self, _pid: u32) -> bool {
                true
            }
            fn prime(&self, pids: &[u32]) {
                self.0.lock().unwrap().push(pids.to_vec());
            }
        }
        let t = TempDir::new().unwrap();
        mk_run(t.path(), "r1", Some("11"), false);
        mk_run(t.path(), "r2", Some("22"), false);
        let rec = Recording(Mutex::new(Vec::new()));
        scan_agent_dir("a", t.path(), &rec).unwrap();
        let calls = rec.0.lock().unwrap();
        assert_eq!(calls.len(), 1, "one prime per scan");
        let mut got = calls[0].clone();
        got.sort_unstable();
        assert_eq!(got, vec![11, 22]);
    }
}
