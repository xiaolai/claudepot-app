//! `claude daemon status` scraper.
//!
//! Surfaces CC's background-supervisor state — running/idle, worker
//! count, sock dir, roster.json path — to Claudepot's UI. Parallel to
//! [`crate::cc_doctor`] but much smaller: the daemon command is a
//! plain line-based dump, not an Ink TUI, so no pty + grid-replay is
//! needed. Plain `Command::output` + line parse.
//!
//! The output format is undocumented (issue #58869). We scrape it
//! anyway because the worker count is otherwise only reachable by
//! reading `roster.json` directly, which the research note
//! (`dev-docs/cc-daemon-research.md`) flags as a more fragile
//! interface than the CLI surface. Both could change, but the CLI
//! at least has user-visible regression pressure.
//!
//! Two fields drive the rest of Claudepot:
//! - `running` + `bg_workers` feed an Activities dashboard tile and
//!   a Sidebar Activity strip badge (render-if-nonzero).
//! - `bg_workers` is plumbed into [`crate::services::usage_snapshot`]
//!   so [`crate::rotation::eval`]'s audit reason can suffix
//!   "(N bg workers active)" — answering the user's "why did
//!   rotation fire when I wasn't even at the keyboard" question.

use serde::{Deserialize, Serialize};
use std::io::Read;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Wall-clock cap on the spawn. `claude daemon status` returns
/// synchronously from a single Unix-socket probe; a 5-second cap
/// allows for cold-start cost on the first invocation after a CC
/// upgrade without blocking a tick loop noticeably.
const SCRAPE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DaemonStatus {
    /// `true` when the supervisor is reachable on its control socket.
    /// Distinguished from "missing roster" — a daemon can be running
    /// with zero workers immediately after `stop --keep-workers`.
    pub running: bool,
    /// Daemon PID when running. `None` in the idle case.
    pub pid: Option<u32>,
    /// Uptime in seconds when running. `None` when idle or when the
    /// uptime field couldn't be parsed.
    pub uptime_secs: Option<u64>,
    /// Active background workers in roster.json. `0` when the daemon
    /// is idle or the roster is absent — distinguished from `None`
    /// (which means "we failed to parse the line at all").
    pub bg_workers: Option<u32>,
    /// `/tmp/cc-daemon-<uid>/<hash>` when present.
    pub sock_dir: Option<PathBuf>,
    /// `/tmp/cc-daemon-<uid>/<hash>/control.sock`. Whether it's
    /// reachable is encoded in `running`; this is the raw path the
    /// daemon advertises.
    pub control_sock: Option<PathBuf>,
    /// Path to roster.json. `None` when the status line said `absent`.
    pub roster_path: Option<PathBuf>,
    /// Path to `~/.claude/daemon.log`. `None` when the status line
    /// said `absent`.
    pub log_path: Option<PathBuf>,
    /// How confident we are in the parse. UI uses this to decide
    /// between "show this" and "show last-known-good" (parallel to
    /// the `cc_doctor` ParseStatus discipline).
    pub parse_status: DaemonParseStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DaemonParseStatus {
    /// `running` and `bg_workers` both parsed cleanly. The other
    /// fields may be `None` individually but the load-bearing pair
    /// is trustworthy.
    Ok,
    /// Output captured but the parser couldn't pin down both `running`
    /// and `bg_workers`. Renderer should fall back to the previous
    /// snapshot rather than show stale-as-fresh.
    Degraded { reason: String },
    /// Spawn or capture failed outright. Same fallback semantics.
    Failed { reason: String },
}

/// Spawn `claude daemon status` and parse the result. Idempotent and
/// cheap — safe to call on the same tick as
/// [`crate::services::usage_snapshot`] writes.
pub fn scrape_daemon_status() -> DaemonStatus {
    match capture_status() {
        Ok(text) => parse_status_output(&text),
        Err(reason) => failed(reason),
    }
}

fn failed(reason: String) -> DaemonStatus {
    DaemonStatus {
        running: false,
        pid: None,
        uptime_secs: None,
        bg_workers: None,
        sock_dir: None,
        control_sock: None,
        roster_path: None,
        log_path: None,
        parse_status: DaemonParseStatus::Failed { reason },
    }
}

fn capture_status() -> Result<String, String> {
    // Reuse cc_doctor's binary resolver so brew-cask / native-install
    // paths work for Tauri-from-Finder launches that don't inherit
    // shell PATH.
    let claude_bin = crate::cc_doctor::probes::resolve_claude_binary()
        .ok_or_else(|| "claude binary not found in canonical install locations".to_string())?;

    // Spawn directly with piped stdio so we own the child handle —
    // a previous mpsc-based version leaked the spawned thread + the
    // claude subprocess when the timeout fired (audit finding,
    // dev-docs/cc-daemon-research.md). On timeout we kill the child
    // and reap it before returning the error.
    let mut child = Command::new(&claude_bin)
        .arg("daemon")
        .arg("status")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("spawn failed: {e}"))?;

    // 50ms poll. CC's daemon status finishes in ~50ms idle; one or
    // two cycles is enough. Polling tighter buys nothing because the
    // child's own work dominates.
    let poll_step = Duration::from_millis(50);
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_status)) => break,
            Ok(None) => {
                if start.elapsed() >= SCRAPE_TIMEOUT {
                    let _ = child.kill();
                    // Best-effort reap so the OS isn't left with a
                    // zombie. Ignore the error — kill already fired.
                    let _ = child.wait();
                    return Err(format!(
                        "status spawn timed out after {}s",
                        SCRAPE_TIMEOUT.as_secs()
                    ));
                }
                std::thread::sleep(poll_step);
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("wait failed: {e}"));
            }
        }
    }

    // Drain pipes after the child has exited. CC daemon status
    // output is sub-1KB so we don't need concurrent draining to
    // avoid pipe-buffer deadlock.
    let mut stdout_buf = String::new();
    let mut stderr_buf = String::new();
    if let Some(mut h) = child.stdout.take() {
        let _ = h.read_to_string(&mut stdout_buf);
    }
    if let Some(mut h) = child.stderr.take() {
        let _ = h.read_to_string(&mut stderr_buf);
    }

    // Idle-daemon exits non-zero ("not running" path), so don't gate
    // on status. Combine stdout+stderr — observed output uses stdout
    // but the CLI is undocumented.
    let mut combined = stdout_buf;
    if !stderr_buf.is_empty() {
        if !combined.is_empty() {
            combined.push('\n');
        }
        combined.push_str(&stderr_buf);
    }
    Ok(combined)
}

/// Pure parser. Takes the full captured text and returns the parsed
/// status. Tested directly with fixture strings — no process spawn.
pub fn parse_status_output(text: &str) -> DaemonStatus {
    let mut out = DaemonStatus {
        running: false,
        pid: None,
        uptime_secs: None,
        bg_workers: None,
        sock_dir: None,
        control_sock: None,
        roster_path: None,
        log_path: None,
        parse_status: DaemonParseStatus::Ok,
    };

    // First non-empty line carries the running/not-running verdict.
    // Observed idle form is the literal string "not running"; the
    // running form is undocumented but the help text says it shows
    // "pid, version, uptime" — match any line that has a digit run
    // after "pid" as a defensive heuristic.
    let mut saw_status_line = false;
    let mut saw_workers_line = false;

    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }

        if !saw_status_line {
            saw_status_line = true;
            if line.eq_ignore_ascii_case("not running")
                || line.to_ascii_lowercase().starts_with("not running")
            {
                out.running = false;
            } else if let Some((pid, uptime)) = parse_running_line(line) {
                out.running = true;
                out.pid = pid;
                out.uptime_secs = uptime;
            } else if line.to_ascii_lowercase().contains("running") {
                // Couldn't extract pid/uptime but the line claims
                // running — record the high-order bit, leave the
                // numeric fields None.
                out.running = true;
            }
            continue;
        }

        // Key-value lines under "bg sessions:". Format observed:
        //   sock dir:     /tmp/cc-daemon-501/<hash>
        //   control.sock: unreachable (...)  | <path>
        //   bg workers:   0 in roster.json (control unreachable)
        //   roster.json:  absent | <path>
        //   daemon.log:   absent | <path>
        if let Some((key, value)) = split_kv(line) {
            match key {
                "sock dir" => out.sock_dir = parse_path_value(value),
                "control.sock" => out.control_sock = parse_path_or_unreachable(value),
                "bg workers" => {
                    saw_workers_line = true;
                    out.bg_workers = parse_worker_count(value);
                }
                "roster.json" => out.roster_path = parse_path_or_absent(value),
                "daemon.log" => out.log_path = parse_path_or_absent(value),
                _ => {}
            }
        }
    }

    // The two load-bearing fields are `running` and `bg_workers`. If
    // we couldn't pin either down, demote to Degraded so the UI keeps
    // the previous snapshot. Anything else missing (paths) is
    // optional.
    if !saw_status_line {
        out.parse_status = DaemonParseStatus::Failed {
            reason: "empty status output".into(),
        };
    } else if !saw_workers_line {
        if is_idle_with_no_section(text) {
            // Clean idle without a "bg sessions:" block — the
            // contract is that an idle daemon reports zero workers,
            // not "unknown." Without this, an old CC version that
            // ever ships a bare "not running" line would surface as
            // Ok-but-None and the badge would correctly hide, but
            // the rotation audit chip would read `None` instead of
            // 0 workers.
            out.bg_workers = Some(0);
        } else {
            out.parse_status = DaemonParseStatus::Degraded {
                reason: "bg workers line missing".into(),
            };
        }
    }

    out
}

/// "not running" + nothing else is the legitimate idle case — treat
/// it as Ok with `bg_workers = Some(0)`. Without this check, idle
/// status would be misreported as Degraded.
fn is_idle_with_no_section(text: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower.contains("not running") && !lower.contains("bg workers")
}

fn parse_running_line(line: &str) -> Option<(Option<u32>, Option<u64>)> {
    // Defensive: look for "pid <digits>" and "uptime <digits>" tokens
    // anywhere on the line. Real format unknown; this matches the
    // help text's promise of "pid, version, uptime" without
    // hard-coding a shape.
    let lower = line.to_ascii_lowercase();
    if !lower.contains("pid") && !lower.contains("running") {
        return None;
    }
    let pid = extract_number_after(&lower, "pid").and_then(|n| u32::try_from(n).ok());
    let uptime = extract_number_after(&lower, "uptime");
    Some((pid, uptime))
}

fn extract_number_after(haystack: &str, key: &str) -> Option<u64> {
    let idx = haystack.find(key)?;
    let rest = &haystack[idx + key.len()..];
    let mut digits = String::new();
    let mut started = false;
    for c in rest.chars() {
        if c.is_ascii_digit() {
            digits.push(c);
            started = true;
        } else if started {
            break;
        } else if c.is_whitespace() || c == ':' || c == '=' {
            continue;
        } else {
            // Non-digit non-separator before any digit — abandon.
            return None;
        }
    }
    digits.parse().ok()
}

fn split_kv(line: &str) -> Option<(&str, &str)> {
    let (k, v) = line.split_once(':')?;
    Some((k.trim(), v.trim()))
}

fn parse_path_value(value: &str) -> Option<PathBuf> {
    let v = value.trim();
    if v.is_empty() || v.eq_ignore_ascii_case("absent") {
        return None;
    }
    Some(PathBuf::from(v))
}

fn parse_path_or_absent(value: &str) -> Option<PathBuf> {
    if value.trim().eq_ignore_ascii_case("absent") {
        return None;
    }
    parse_path_value(value)
}

fn parse_path_or_unreachable(value: &str) -> Option<PathBuf> {
    let v = value.trim();
    if v.to_ascii_lowercase().starts_with("unreachable") {
        // Try to recover the sock path from the parenthesized hint:
        //   "unreachable (connect ENOENT /tmp/.../control.sock)"
        if let (Some(open), Some(close)) = (v.find('('), v.rfind(')')) {
            if close > open {
                let inner = &v[open + 1..close];
                // Pull the last whitespace-separated token that looks
                // like an absolute path.
                if let Some(last) = inner
                    .split_whitespace()
                    .rev()
                    .find(|tok| tok.starts_with('/'))
                {
                    return Some(PathBuf::from(last));
                }
            }
        }
        return None;
    }
    parse_path_value(value)
}

fn parse_worker_count(value: &str) -> Option<u32> {
    // Format: "N in roster.json (...)" or just "N". Pull the first
    // digit run.
    let mut digits = String::new();
    for c in value.chars() {
        if c.is_ascii_digit() {
            digits.push(c);
        } else if !digits.is_empty() {
            break;
        }
    }
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDLE_FIXTURE: &str = "\
not running

bg sessions:
  sock dir:     /tmp/cc-daemon-501/5efc884f
  control.sock: unreachable (connect ENOENT /tmp/cc-daemon-501/5efc884f/control.sock)
  bg workers:   0 in roster.json (control unreachable)
  roster.json:  absent
  daemon.log:   absent
";

    #[test]
    fn idle_fixture_parses_to_zero_workers() {
        let s = parse_status_output(IDLE_FIXTURE);
        assert!(!s.running);
        assert_eq!(s.bg_workers, Some(0));
        assert_eq!(
            s.sock_dir.as_deref(),
            Some(std::path::Path::new("/tmp/cc-daemon-501/5efc884f"))
        );
        assert_eq!(
            s.control_sock.as_deref(),
            Some(std::path::Path::new(
                "/tmp/cc-daemon-501/5efc884f/control.sock"
            ))
        );
        assert_eq!(s.roster_path, None);
        assert_eq!(s.log_path, None);
        assert!(matches!(s.parse_status, DaemonParseStatus::Ok));
    }

    #[test]
    fn running_with_workers_parses_count() {
        // Hypothetical running form. Test the parser, not the live CLI.
        let fixture = "\
running pid 12345 uptime 3600

bg sessions:
  sock dir:     /tmp/cc-daemon-501/abc
  control.sock: /tmp/cc-daemon-501/abc/control.sock
  bg workers:   3 in roster.json
  roster.json:  /Users/me/.claude/daemon/roster.json
  daemon.log:   /Users/me/.claude/daemon.log
";
        let s = parse_status_output(fixture);
        assert!(s.running);
        assert_eq!(s.pid, Some(12345));
        assert_eq!(s.uptime_secs, Some(3600));
        assert_eq!(s.bg_workers, Some(3));
        assert!(s.roster_path.is_some());
        assert!(s.log_path.is_some());
        assert!(matches!(s.parse_status, DaemonParseStatus::Ok));
    }

    #[test]
    fn bare_not_running_no_section_parses_clean() {
        // Some future CC version may drop the "bg sessions:" block
        // entirely when idle. Contract: clean idle reports Some(0),
        // not None — "we measured and it's zero" beats "we don't know".
        let s = parse_status_output("not running\n");
        assert!(!s.running);
        assert_eq!(s.bg_workers, Some(0));
        assert!(matches!(s.parse_status, DaemonParseStatus::Ok));
    }

    #[test]
    fn empty_output_is_failed() {
        let s = parse_status_output("");
        assert!(matches!(s.parse_status, DaemonParseStatus::Failed { .. }));
    }

    #[test]
    fn missing_workers_line_when_section_present_is_degraded() {
        let fixture = "\
running

bg sessions:
  sock dir:     /tmp/cc-daemon-501/abc
";
        let s = parse_status_output(fixture);
        assert!(s.running);
        assert!(matches!(
            s.parse_status,
            DaemonParseStatus::Degraded { .. }
        ));
    }

    #[test]
    fn unreachable_recovers_sock_path_from_parens() {
        let s = parse_status_output(IDLE_FIXTURE);
        // Even though control.sock said "unreachable", the embedded
        // path is recovered from the parenthesized hint so the UI can
        // still show "expected at <path>".
        assert!(s.control_sock.is_some());
    }

    #[test]
    fn worker_count_handles_extra_text() {
        assert_eq!(parse_worker_count("5 in roster.json (whatever)"), Some(5));
        assert_eq!(parse_worker_count("0"), Some(0));
        assert_eq!(parse_worker_count("none"), None);
    }

    #[test]
    fn extract_number_after_finds_uptime() {
        assert_eq!(
            extract_number_after("pid 123 uptime 9876 seconds", "uptime"),
            Some(9876)
        );
        assert_eq!(extract_number_after("pid 123", "uptime"), None);
    }

    #[test]
    fn path_or_absent_returns_none_for_absent() {
        assert_eq!(parse_path_or_absent("absent"), None);
        assert_eq!(parse_path_or_absent("Absent"), None);
        assert_eq!(
            parse_path_or_absent("/Users/me/.claude/daemon.log"),
            Some(PathBuf::from("/Users/me/.claude/daemon.log"))
        );
    }

    #[test]
    #[ignore = "live: spawns real `claude daemon status`, requires CC installed"]
    fn live_scrape_against_real_claude() {
        let s = scrape_daemon_status();
        eprintln!(
            "live: running={} pid={:?} workers={:?} parse_status={:?}",
            s.running, s.pid, s.bg_workers, s.parse_status
        );
        // No hard assert on `running` — we don't know whether the
        // user has a daemon up. Assert only that the parser didn't
        // outright fail.
        assert!(!matches!(s.parse_status, DaemonParseStatus::Failed { .. }));
    }
}
