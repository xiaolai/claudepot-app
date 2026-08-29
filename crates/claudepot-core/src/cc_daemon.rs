//! CC background-daemon status, read from `roster.json`.
//!
//! Surfaces CC's background-supervisor state — running, and how many
//! background workers it holds — to Claudepot's UI. Two consumers:
//!
//! - `SidebarBgBadge` renders the worker count, render-if-nonzero.
//! - `bg_workers` is plumbed into [`crate::services::usage_snapshot`]
//!   so [`crate::rotation::eval`]'s audit reason can suffix
//!   "(N bg workers active)" — answering the user's "why did rotation
//!   fire when I wasn't even at the keyboard" question.
//!
//! # Why this does not run `claude daemon status`
//!
//! It used to, once a minute, and that was issue #94.
//!
//! `claude`'s grammar is `claude [options] [command] [prompt]`, and an
//! unrecognized **positional** becomes the prompt rather than an
//! error. On a binary predating the `daemon` subcommand (CC 2.1.139)
//! `claude daemon status` therefore did not fail — it started a
//! headless session whose prompt was the word `daemon`, called the
//! model, and billed for it. A reporter running CC 2.1.39 measured
//! **288 such sessions in one day at ~20K uncached input tokens
//! each**, rendering nothing: the scrape was "infallible by design",
//! so every failure degraded to a hidden badge. Anthropic fixed a
//! sibling case of the same fall-through upstream at CC 2.1.199.
//!
//! A version floor ([`crate::cc_capability`]) fixes that instance and
//! **not** the mechanism: a floor guards the lower bound only, so a
//! future CC that renames or removes `daemon` walks through
//! `>= 2.1.139` and bills again. For data polled on a timer, the only
//! durable answer is to not reach the CLI at all. Hence this module
//! reads a file.
//!
//! # Why reading the file is the better failure mode, not the safer interface
//!
//! `dev-docs/cc-daemon-research.md` §7c.2 recorded the opposite rule —
//! *don't scrape `roster.json`, the CLI carries user-visible
//! regression pressure*. That compared the two interfaces on
//! **fragility**, where the CLI genuinely wins. It is the wrong axis.
//! Both are undocumented and both will drift; what differs is the
//! **cost of being wrong**. A `roster.json` schema change costs a
//! degraded badge. A CLI spawn against the wrong binary costs money,
//! silently, forever. That asymmetry decides it, and it is why the
//! rule is now inverted (see the research note's amendment).
//!
//! Two things make the file the *sounder* read anyway:
//!
//! - it carries `proto`, a schema version it declares about itself,
//!   which the line-oriented CLI output never did; and
//! - it is one `read` of a ~100-byte file rather than spawning a
//!   197 MB binary, which is what a once-a-minute badge should cost.
//!
//! # Why there is no `running` flag, and why the count is per worker
//!
//! The obvious reading of "is the daemon up" is `supervisorPid`'s
//! liveness, and this module shipped that for one revision. It is not
//! sound: the roster's schema carries **no `procStart` for the
//! supervisor**, so a recycled PID reads as a live daemon — and since
//! the roster survives the daemon by design (13 days, on the machine
//! this was measured on) a stale roster's dead workers would then be
//! reported as active. A fabricated number on a badge is the precise
//! failure this module criticises the old CLI parser for.
//!
//! Every *worker* record, by contrast, carries `pid` **and**
//! `startedAt`, both required. So the guarded question is asked per
//! worker — is this pid alive, and did it begin no later than the
//! roster says it did — via [`crate::agent::liveness::ProcessCheck`],
//! which already implements exactly that comparison for recycled
//! `run.pid`s. The count that renders is fully guarded, and the
//! boolean that could not be is simply not reported. A worker that
//! outlives its supervisor is a live worker either way; CC prints a
//! warning for that state rather than calling it idle.
//!
//! # Shape, read out of the 2.1.251 binary
//!
//! CC's own zod schema and path helpers:
//!
//! ```text
//! function c(){return i(be(),"daemon")}          // <configDir>/daemon
//! function fDe(){return Te.daemon(["roster.json"])}
//! un({ proto:        v().int().min(1).max(1),
//!      supervisorPid: v().catch(0),
//!      updatedAt:     v().catch(0),
//!      workers:       De(<short id>, {
//!          pid: v(), procStart: i().optional(), sessionId: i(),
//!          cliVersion: i().optional(), startedAt: v(), attempt: v(),
//!          cwd: i(), … }) })
//! ```
//!
//! `pid` and `startedAt` are the two required fields, which is why
//! they are the pair this module reads. `procStart` would also serve
//! and is the wrong choice: it is optional, and it is a
//! `ps -o lstart=` string with a separate Windows shape, where
//! `startedAt` is epoch milliseconds and needs no subprocess on
//! either platform.
//!
//! Note the `.catch(0)` on the scalars: CC tolerates a garbage field
//! without discarding the rest of the file, and so does the reader
//! below. A roster whose `supervisorPid` is unreadable must still
//! yield a worker count.

use crate::agent::liveness::ProcessCheck;
use crate::session_live::registry::SysinfoCheck;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Largest roster we will read.
///
/// **1 MiB, not CC's 8 MiB.** CC quarantines its own roster above 8
/// MiB, so anything past that is a file CC has already disowned. But
/// `.claude/rules/rust-conventions.md` allows a synchronous
/// `std::fs` read only for files under 1 MB, and this read is
/// synchronous by design — both callers wrap it in `spawn_blocking`.
/// Rather than argue the rule down, the reader takes the tighter
/// bound.
///
/// The window between the two is not a real roster. Worker records run
/// a few hundred bytes each, so 1 MiB is on the order of two thousand
/// concurrent background sessions; a file that size is corruption, and
/// refusing it costs a badge rather than a wrong number.
const MAX_ROSTER_BYTES: u64 = 1024 * 1024;

/// The `proto` range this build knows how to read. CC 2.1.251 pins its
/// own accepted range to `[1, 1]`, and this must not be wider: an
/// earlier revision gated on `p <= MAX` alone, which accepted `proto:
/// 0` — a value CC itself rejects — and went on to report a worker
/// count from a schema nobody has ever defined.
///
/// A roster declaring a higher proto is reported [`DaemonParseStatus::Degraded`]
/// with **no counts**, rather than read optimistically. The fields are
/// simple enough that a newer proto would probably still parse, and
/// "probably" is how a fabricated number reaches a badge — the same
/// failure the old CLI parser shipped when it stored the sentence
/// `updated 765423s ago` into a `PathBuf`.
const MIN_KNOWN_PROTO: u64 = 1;
const MAX_KNOWN_PROTO: u64 = 1;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DaemonStatus {
    /// Background workers whose process is **actually alive**, not the
    /// roster's length. `None` means "could not tell" and must not be
    /// rendered as a count; `Some(0)` is the healthy idle answer.
    ///
    /// Each entry is checked against its own `startedAt`, so a roster
    /// left behind by a dead daemon contributes nothing however long
    /// it sits there, and a recycled PID is not mistaken for the
    /// worker that used to own it.
    pub bg_workers: Option<u32>,
    /// The roster file consulted. Present whether or not it existed,
    /// because "which file did you look at" is the first question of
    /// anyone debugging a wrong count.
    pub roster_path: Option<PathBuf>,
    /// How much of the read to trust. UI uses this to choose between
    /// "show this" and "keep the last good snapshot" (the same
    /// discipline as [`crate::cc_doctor`]).
    pub parse_status: DaemonParseStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DaemonParseStatus {
    /// `bg_workers` is trustworthy.
    Ok,
    /// The file was read but something in it could not be pinned down.
    /// Renderer should fall back to the previous snapshot rather than
    /// show stale-as-fresh.
    Degraded { reason: String },
    /// The file exists and could not be read or parsed at all. Same
    /// fallback semantics.
    Failed { reason: String },
}

/// Where CC keeps the roster: `<config dir>/daemon/roster.json`.
///
/// Resolved through [`crate::paths::claude_config_dir`], which honours
/// `CLAUDE_CONFIG_DIR` exactly as CC's own `be()` does. Hard-coding
/// the `~/.claude` sibling is the bug `cc_tips::history` shipped — it
/// reported `num_startups: 0` forever under a custom config dir.
pub fn roster_path() -> PathBuf {
    crate::paths::claude_config_dir()
        .join("daemon")
        .join("roster.json")
}

/// Read the daemon status. Cheap, spawns nothing, and safe to call on
/// the same tick as [`crate::services::usage_snapshot`] writes.
pub fn daemon_status() -> DaemonStatus {
    read_daemon_status_at(&roster_path(), &SysinfoCheck::new())
}

/// Seam for tests: explicit roster path and process check.
pub fn read_daemon_status_at(path: &Path, procs: &dyn ProcessCheck) -> DaemonStatus {
    let base = DaemonStatus {
        bg_workers: None,
        roster_path: Some(path.to_path_buf()),
        parse_status: DaemonParseStatus::Ok,
    };

    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // The documented idle state. CC prints `roster.json:
            // absent` here and calls the daemon not running, so this
            // is a clean zero and not an unknown — without that, a
            // machine that has never started a bg session would show
            // the rotation audit a `None` instead of 0.
            return DaemonStatus {
                bg_workers: Some(0),
                ..base
            };
        }
        Err(e) => {
            return DaemonStatus {
                parse_status: DaemonParseStatus::Failed {
                    reason: format!("could not stat roster.json: {e}"),
                },
                ..base
            };
        }
    };

    if !meta.is_file() {
        return DaemonStatus {
            parse_status: DaemonParseStatus::Failed {
                reason: "roster.json is not a regular file".into(),
            },
            ..base
        };
    }
    if meta.len() > MAX_ROSTER_BYTES {
        return DaemonStatus {
            parse_status: DaemonParseStatus::Failed {
                reason: format!(
                    "roster.json is {} bytes, past the {MAX_ROSTER_BYTES}-byte bound \
                     CC quarantines its own roster at",
                    meta.len()
                ),
            },
            ..base
        };
    }

    let raw = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) => {
            return DaemonStatus {
                parse_status: DaemonParseStatus::Failed {
                    reason: format!("could not read roster.json: {e}"),
                },
                ..base
            };
        }
    };

    interpret_roster(&raw, procs, base)
}

/// The whole judgement, with no I/O. Field extraction is per-field and
/// tolerant, mirroring CC's `.catch(0)`: one unreadable scalar must
/// not throw away a worker count that parsed fine.
fn interpret_roster(raw: &str, procs: &dyn ProcessCheck, base: DaemonStatus) -> DaemonStatus {
    let value: serde_json::Value = match serde_json::from_str(raw) {
        Ok(v) => v,
        Err(e) => {
            return DaemonStatus {
                parse_status: DaemonParseStatus::Failed {
                    reason: format!("roster.json is not valid JSON: {e}"),
                },
                ..base
            };
        }
    };

    let Some(obj) = value.as_object() else {
        return DaemonStatus {
            parse_status: DaemonParseStatus::Failed {
                reason: "roster.json is not a JSON object".into(),
            },
            ..base
        };
    };

    // `proto` gates everything below it. A roster that does not
    // declare one is not a shape we recognise, and a roster newer than
    // we understand is read by nobody here — both keep their counts to
    // themselves rather than guessing.
    match obj.get("proto").and_then(serde_json::Value::as_u64) {
        Some(p) if (MIN_KNOWN_PROTO..=MAX_KNOWN_PROTO).contains(&p) => {}
        Some(p) => {
            return DaemonStatus {
                parse_status: DaemonParseStatus::Degraded {
                    reason: format!(
                        "roster.json declares proto {p}; this build reads \
                         {MIN_KNOWN_PROTO}..={MAX_KNOWN_PROTO}"
                    ),
                },
                ..base
            };
        }
        None => {
            return DaemonStatus {
                parse_status: DaemonParseStatus::Degraded {
                    reason: "roster.json has no readable `proto` field".into(),
                },
                ..base
            };
        }
    }

    let Some(workers) = obj.get("workers").and_then(serde_json::Value::as_object) else {
        return DaemonStatus {
            parse_status: DaemonParseStatus::Degraded {
                reason: "roster.json has no readable `workers` object".into(),
            },
            ..base
        };
    };

    // `pid` and `startedAt` are both REQUIRED in CC's worker schema, so
    // a record missing either is drift rather than an odd worker. That
    // degrades the whole read: a count computed over the entries we
    // happened to understand is exactly the number nobody computed.
    let mut entries: Vec<Worker> = Vec::with_capacity(workers.len());
    for (id, w) in workers {
        let pid = w
            .get("pid")
            .and_then(serde_json::Value::as_u64)
            .and_then(|n| u32::try_from(n).ok())
            .filter(|pid| *pid != 0);
        let started_at_ms = w.get("startedAt").and_then(serde_json::Value::as_i64);
        match (pid, started_at_ms) {
            (Some(pid), Some(started_at_ms)) => entries.push(Worker { pid, started_at_ms }),
            _ => {
                return DaemonStatus {
                    parse_status: DaemonParseStatus::Degraded {
                        reason: format!(
                            "roster worker {id} has no readable `pid` + `startedAt` pair"
                        ),
                    },
                    ..base
                };
            }
        }
    }

    // One process-table refresh for the whole roster rather than one
    // per worker.
    let pids: Vec<u32> = entries.iter().map(|w| w.pid).collect();
    procs.prime(&pids);

    let live = entries
        .iter()
        .filter(|w| procs.is_running(w.pid) && !procs.started_after(w.pid, w.started_at_ms))
        .count();

    DaemonStatus {
        bg_workers: Some(u32::try_from(live).unwrap_or(u32::MAX)),
        parse_status: DaemonParseStatus::Ok,
        ..base
    }
}

/// The two required fields of a roster worker record.
struct Worker {
    pid: u32,
    /// Epoch **milliseconds**, as CC writes it. A process whose start
    /// time is later than this is a recycled pid, not this worker —
    /// see [`ProcessCheck::started_after`], whose "cannot tell" answer
    /// is `false` so an unreadable start time preserves the
    /// pid-exists verdict rather than inventing a recycle.
    started_at_ms: i64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// Process table under test control. `started_secs` lets a test
    /// express the case the whole guard exists for: a pid that is
    /// alive but began *after* the roster recorded the worker, i.e. a
    /// recycled number wearing a dead worker's pid.
    struct FakeProcs {
        started_secs: Mutex<HashMap<u32, u64>>,
        primed: Mutex<Vec<u32>>,
    }

    impl FakeProcs {
        /// Every pid alive and started at epoch 0 — old enough that no
        /// realistic `startedAt` makes it look recycled.
        fn alive(pids: &[u32]) -> Self {
            Self {
                started_secs: Mutex::new(pids.iter().map(|p| (*p, 0)).collect()),
                primed: Mutex::new(Vec::new()),
            }
        }
        fn none() -> Self {
            Self::alive(&[])
        }
        /// Alive, but started at `secs` — used to fake pid reuse.
        fn started_at(pid: u32, secs: u64) -> Self {
            Self {
                started_secs: Mutex::new([(pid, secs)].into_iter().collect()),
                primed: Mutex::new(Vec::new()),
            }
        }
    }

    impl ProcessCheck for FakeProcs {
        fn is_running(&self, pid: u32) -> bool {
            self.started_secs.lock().unwrap().contains_key(&pid)
        }
        fn prime(&self, pids: &[u32]) {
            self.primed.lock().unwrap().extend_from_slice(pids);
        }
        fn started_after(&self, pid: u32, since_ms: i64) -> bool {
            match self.started_secs.lock().unwrap().get(&pid) {
                // Same strict-seconds comparison the real impl uses.
                Some(secs) => i64::try_from(*secs).is_ok_and(|s| s > since_ms / 1000),
                None => false,
            }
        }
    }

    fn base() -> DaemonStatus {
        DaemonStatus {
            bg_workers: None,
            roster_path: Some(PathBuf::from("/fake/roster.json")),
            parse_status: DaemonParseStatus::Ok,
        }
    }

    fn read(raw: &str, procs: &dyn ProcessCheck) -> DaemonStatus {
        interpret_roster(raw, procs, base())
    }

    /// One worker, `startedAt` far in the future so a fake pid started
    /// at epoch 0 always reads as legitimate.
    fn roster(workers: &[(&str, u32)]) -> String {
        let body: Vec<String> = workers
            .iter()
            .map(|(id, pid)| format!(r#""{id}":{{"pid":{pid},"startedAt":9000000000000}}"#))
            .collect();
        format!(
            r#"{{"proto":1,"supervisorPid":4242,"updatedAt":1,"workers":{{{}}}}}"#,
            body.join(",")
        )
    }

    /// The exact bytes on the reference machine, whose `claude daemon
    /// status` printed `not running` / `bg workers: 0 in roster.json`.
    const REAL_IDLE_ROSTER: &str = r#"{
  "proto": 1,
  "supervisorPid": 22163,
  "updatedAt": 1786816263045,
  "workers": {}
}"#;

    #[test]
    fn reproduces_the_cli_verdict_on_a_real_idle_roster() {
        let s = read(REAL_IDLE_ROSTER, &FakeProcs::none());
        assert_eq!(s.bg_workers, Some(0));
        assert_eq!(s.parse_status, DaemonParseStatus::Ok);
    }

    #[test]
    fn counts_workers_whose_processes_are_alive() {
        let s = read(
            &roster(&[("aa", 11), ("bb", 22), ("cc", 33)]),
            &FakeProcs::alive(&[11, 22, 33]),
        );
        assert_eq!(s.bg_workers, Some(3));
        assert_eq!(s.parse_status, DaemonParseStatus::Ok);
    }

    #[test]
    fn a_dead_worker_is_not_counted() {
        // Two entries, one process. The roster length is 2 and the
        // honest answer is 1 — the whole reason the count is not
        // `workers.len()`.
        let s = read(&roster(&[("aa", 11), ("bb", 22)]), &FakeProcs::alive(&[11]));
        assert_eq!(s.bg_workers, Some(1));
    }

    /// The state that motivated this design: CC's roster outlives the
    /// daemon by design — 13 days, measured — so every entry in a
    /// stale roster is dead however healthy the file looks.
    #[test]
    fn a_stale_roster_contributes_nothing() {
        let s = read(&roster(&[("aa", 11), ("bb", 22)]), &FakeProcs::none());
        assert_eq!(s.bg_workers, Some(0));
    }

    /// The reuse guard itself. The pid is ALIVE — so a bare
    /// `is_running` check would count it — but it began after the
    /// roster recorded this worker, which makes it a different
    /// process wearing a recycled number.
    #[test]
    fn a_recycled_pid_is_not_the_worker_that_owned_it() {
        let raw = r#"{"proto":1,"supervisorPid":1,"updatedAt":1,
                      "workers":{"aa":{"pid":77,"startedAt":1000000}}}"#;
        let procs = FakeProcs::started_at(77, 9_000_000);
        assert!(procs.is_running(77), "the fake must keep the pid alive");
        assert_eq!(
            read(raw, &procs).bg_workers,
            Some(0),
            "a pid that began after its roster entry is a recycled number"
        );
    }

    #[test]
    fn a_process_started_before_its_roster_entry_is_the_real_worker() {
        // CC records `startedAt` after spawning, so a legitimate
        // worker's process always predates its entry. An off-by-one
        // here would report every live worker as recycled.
        let raw = r#"{"proto":1,"supervisorPid":1,"updatedAt":1,
                      "workers":{"aa":{"pid":77,"startedAt":9000000}}}"#;
        assert_eq!(
            read(raw, &FakeProcs::started_at(77, 1_000)).bg_workers,
            Some(1)
        );
    }

    #[test]
    fn primes_the_process_table_with_every_worker_pid() {
        // SysinfoCheck answers `false` for any pid it was never primed
        // with, so a missing prime reports every live worker as dead.
        let procs = FakeProcs::alive(&[11, 22]);
        read(&roster(&[("aa", 11), ("bb", 22)]), &procs);
        let mut primed = procs.primed.lock().unwrap().clone();
        primed.sort_unstable();
        assert_eq!(primed, vec![11, 22]);
    }

    #[test]
    fn a_worker_missing_its_required_fields_degrades() {
        // `pid` and `startedAt` are both required in CC's schema, so a
        // record without them is drift. Counting the rest would be a
        // number computed over an arbitrary subset.
        for raw in [
            r#"{"proto":1,"updatedAt":1,"workers":{"aa":{"startedAt":1}}}"#,
            r#"{"proto":1,"updatedAt":1,"workers":{"aa":{"pid":11}}}"#,
            r#"{"proto":1,"updatedAt":1,"workers":{"aa":{"pid":0,"startedAt":1}}}"#,
        ] {
            let s = read(raw, &FakeProcs::alive(&[11]));
            assert!(
                matches!(s.parse_status, DaemonParseStatus::Degraded { .. }),
                "expected Degraded for {raw}, got {:?}",
                s.parse_status
            );
            assert_eq!(s.bg_workers, None, "a degraded read must yield no count");
        }
    }

    #[test]
    fn a_newer_proto_degrades_and_reports_no_numbers() {
        let s = read(
            r#"{"proto":2,"updatedAt":1,"workers":{"a":{"pid":11,"startedAt":1}}}"#,
            &FakeProcs::alive(&[11]),
        );
        assert!(
            matches!(s.parse_status, DaemonParseStatus::Degraded { .. }),
            "got {:?}",
            s.parse_status
        );
        assert_eq!(
            s.bg_workers, None,
            "a proto we cannot read must not yield a count — that is how a \
             fabricated number reaches the badge"
        );
    }

    #[test]
    fn proto_zero_degrades_even_though_it_is_below_the_ceiling() {
        // `p <= MAX` alone accepted this. CC's own range is [1,1], so
        // proto 0 is a schema nobody defined — reporting a count from
        // it is the fabricated-number failure in a new costume.
        let s = read(
            r#"{"proto":0,"updatedAt":1,"workers":{"a":{"pid":11,"startedAt":1}}}"#,
            &FakeProcs::alive(&[11]),
        );
        assert!(
            matches!(s.parse_status, DaemonParseStatus::Degraded { .. }),
            "got {:?}",
            s.parse_status
        );
        assert_eq!(s.bg_workers, None);
    }

    #[test]
    fn a_missing_proto_degrades() {
        let s = read(r#"{"updatedAt":1,"workers":{}}"#, &FakeProcs::none());
        assert!(matches!(s.parse_status, DaemonParseStatus::Degraded { .. }));
        assert_eq!(s.bg_workers, None);
    }

    #[test]
    fn a_missing_workers_object_degrades_rather_than_counting_zero() {
        // "I could not find the workers" and "there are no workers"
        // are different answers; collapsing them would report a
        // healthy idle daemon over a roster we failed to read.
        let s = read(
            r#"{"proto":1,"supervisorPid":4242,"updatedAt":1}"#,
            &FakeProcs::none(),
        );
        assert!(matches!(s.parse_status, DaemonParseStatus::Degraded { .. }));
        assert_eq!(s.bg_workers, None);
    }

    #[test]
    fn malformed_json_fails_rather_than_degrading() {
        let s = read("{not json", &FakeProcs::none());
        assert!(matches!(s.parse_status, DaemonParseStatus::Failed { .. }));
        assert_eq!(s.bg_workers, None);
    }

    #[test]
    fn a_json_scalar_is_not_a_roster() {
        let s = read("42", &FakeProcs::none());
        assert!(matches!(s.parse_status, DaemonParseStatus::Failed { .. }));
    }

    #[test]
    fn an_absent_roster_is_the_idle_state_not_a_failure() {
        let dir = tempfile::tempdir().unwrap();
        let s = read_daemon_status_at(&dir.path().join("roster.json"), &FakeProcs::none());
        assert_eq!(s.parse_status, DaemonParseStatus::Ok);
        assert_eq!(
            s.bg_workers,
            Some(0),
            "a machine that never started a bg session reports zero, not unknown"
        );
    }

    #[test]
    fn an_oversized_roster_is_refused_unread() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("roster.json");
        std::fs::write(&p, vec![b'x'; (MAX_ROSTER_BYTES + 1) as usize]).unwrap();
        let s = read_daemon_status_at(&p, &FakeProcs::none());
        assert!(matches!(s.parse_status, DaemonParseStatus::Failed { .. }));
    }

    #[test]
    fn a_directory_where_the_roster_should_be_is_a_failure() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("roster.json");
        std::fs::create_dir(&p).unwrap();
        let s = read_daemon_status_at(&p, &FakeProcs::none());
        assert!(matches!(s.parse_status, DaemonParseStatus::Failed { .. }));
    }

    #[test]
    fn the_roster_path_is_always_reported() {
        // "which file did you read" is the first debugging question,
        // and it must be answerable in every branch — including the
        // ones that read nothing.
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("roster.json");
        assert_eq!(
            read_daemon_status_at(&p, &FakeProcs::none()).roster_path,
            Some(p.clone())
        );
        std::fs::write(&p, "{bad").unwrap();
        assert_eq!(
            read_daemon_status_at(&p, &FakeProcs::none()).roster_path,
            Some(p)
        );
    }

    #[test]
    fn roster_path_follows_the_cc_config_dir_override() {
        // Hard-coding the `~/.claude` sibling is the bug
        // `cc_tips::history` shipped. Guard it here rather than
        // rediscover it.
        let _lock = crate::testing::lock_data_dir();
        let dir = tempfile::tempdir().unwrap();
        // Restore rather than remove: a developer running the suite
        // with a real CLAUDE_CONFIG_DIR set should not have it wiped
        // for every test that follows.
        let saved = std::env::var_os("CLAUDE_CONFIG_DIR");
        std::env::set_var("CLAUDE_CONFIG_DIR", dir.path());
        let got = roster_path();
        match saved {
            Some(v) => std::env::set_var("CLAUDE_CONFIG_DIR", v),
            None => std::env::remove_var("CLAUDE_CONFIG_DIR"),
        }
        assert_eq!(got, dir.path().join("daemon").join("roster.json"));
    }
}
