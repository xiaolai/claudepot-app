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

use crate::proc_utils::NoWindowExt;
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
    /// uptime field couldn't be parsed. Read from the `uptime:  <N>s`
    /// line of the running-daemon header — it is not on the first
    /// line, which is why this was always `None` before.
    pub uptime_secs: Option<u64>,
    /// Active background workers in roster.json. `0` when the daemon
    /// is idle or the roster is absent — distinguished from `None`
    /// (which means "we failed to parse the line at all").
    pub bg_workers: Option<u32>,
    /// `/tmp/cc-daemon-<uid>/<hash>` when present.
    pub sock_dir: Option<PathBuf>,
    /// `/tmp/cc-daemon-<uid>/<hash>/control.sock` when CC names it.
    /// `None` when the socket answered — CC prints the word
    /// `reachable` in place of the path in that case, so a healthy
    /// daemon yields `None` here and `running: true`. When the socket
    /// is *un*reachable the path is recovered from the error hint,
    /// which is the case a UI would want to show.
    pub control_sock: Option<PathBuf>,
    /// Path to roster.json, when CC advertises one. **Always `None` on
    /// CC 2.1.241** — that line now reports the roster's age rather
    /// than its location. Retained for older CC, which printed a path.
    pub roster_path: Option<PathBuf>,
    /// Path to `~/.claude/daemon.log`. `None` when the status line
    /// said `absent`, i.e. the log file does not exist yet.
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
        .no_window()
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
///
/// # The format
///
/// Read out of CC 2.1.241's `formatBgDaemonStatus` and the `status`
/// arm of the `daemon` command, not sampled from one run — a sample
/// only shows the branches that machine happened to be in, which is
/// how three of these lines were modelled wrongly. Optional lines are
/// bracketed; `|` separates alternatives of one line.
///
/// ```text
/// not running                                        # idle: first line
/// pid:     <N>                                       # running: first line
/// version: <ver>
/// uptime:  <N>s
/// origin:  <origin>
/// config:  <path>
/// log:     <path>
/// [launcher: <cmd> | (none)]
/// [warning: <...>]
///
/// bg sessions:
///   sock dir:     <path>
///   control.sock: reachable | unreachable (<err>)
///   [bg sessions:  disabled (start failure — ...)]
///   bg workers:   <live> running (control.sock), <roster> in roster.json
///               | <roster> in roster.json (live count unavailable | control unreachable)
///   [              <N> from a different CLI version (...)]
///   roster.json:  absent | updated <N>s ago
///   daemon.log:   absent | <size> at <path>
///   [warning:      supervisor not running but <N> workers in roster — ...]
/// ```
///
/// Three of those lines print a **sentence where an older CC printed a
/// path**, and the parser used to store the sentence as a `PathBuf`:
/// `control.sock: reachable`, `roster.json: updated <N>s ago`, and
/// `daemon.log: <size> at <path>` became `control_sock = "reachable"`,
/// `roster_path = "updated 765423s ago"` and
/// `log_path = "18.3KB at /Users/…/daemon.log"`. Every path arm now
/// goes through [`parse_absolute_path_or_none`], so an unrecognized
/// shape yields `None` — the honest answer — instead of a fabricated
/// path that `.claude/rules/path-display.md` would then render.
///
/// Note there is **no `running` line**: a running daemon announces
/// itself by leading with `pid:`, so that is what the first-line
/// branch has to recognize.
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
    // Idle is the literal "not running"; running leads with
    // `pid:     <N>`. `parse_running_line`'s digit-after-"pid" search
    // covers both that shape and any future one-line variant.
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

        // Key-value lines. See the format block in `parse_status_output`'s
        // doc comment for the authoritative shapes; every arm below
        // refuses to invent a path out of prose.
        if let Some((key, value)) = split_kv(line) {
            match key {
                "sock dir" => out.sock_dir = parse_path_value(value),
                "control.sock" => out.control_sock = parse_control_sock(value),
                "bg workers" => {
                    saw_workers_line = true;
                    out.bg_workers = parse_worker_count(value);
                }
                "roster.json" => out.roster_path = parse_roster_value(value),
                "daemon.log" => out.log_path = parse_log_value(value),
                // Running-daemon header block: `uptime:  <N>s`. The
                // first-line branch above never sees it, because when
                // the daemon runs the first line is `pid:     <N>`.
                "uptime" => out.uptime_secs = out.uptime_secs.or_else(|| parse_uptime(value)),
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

/// `roster.json:  absent | updated <N>s ago | <path>`.
///
/// Current CC never prints a path here — the line reports *freshness*,
/// not location. The old shape is still accepted so an older CC keeps
/// working, but anything that is not an absolute path yields `None`
/// rather than a `PathBuf` built out of the sentence "updated 765423s
/// ago". The age itself is deliberately dropped: no surface renders
/// it, and a field with no reader is worse than no field.
fn parse_roster_value(value: &str) -> Option<PathBuf> {
    parse_absolute_path_or_none(value)
}

/// `daemon.log:   absent | <size> at <path> | <path>`.
///
/// The size prefix is a formatted string (`18.3KB`), so it is read for
/// its `" at "` separator only and then discarded — parsing it back to
/// a byte count would be lossy. Split on the FIRST `" at "`: the size
/// never contains one, and a path legitimately can.
fn parse_log_value(value: &str) -> Option<PathBuf> {
    let v = value.trim();
    if let Some((_size, rest)) = v.split_once(" at ") {
        return parse_absolute_path_or_none(rest);
    }
    parse_absolute_path_or_none(v)
}

/// The one gate that stops prose becoming a path. `absent`,
/// `reachable`, `updated 765423s ago` and any future sentence all fall
/// through to `None`; only something path-shaped survives.
///
/// `is_absolute_path_str` (not `starts_with('/')`, not
/// `Path::is_absolute` — .claude/rules/paths.md) so Unix, drive-letter,
/// UNC and named-pipe shapes are all recognized on every host.
fn parse_absolute_path_or_none(value: &str) -> Option<PathBuf> {
    let v = value.trim();
    if v.is_empty() || !crate::path_utils::is_absolute_path_str(v) {
        return None;
    }
    Some(PathBuf::from(v))
}

/// `uptime:  <N>s` from the running-daemon header block.
fn parse_uptime(value: &str) -> Option<u64> {
    let digits: String = value
        .trim()
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

/// `control.sock: reachable | unreachable (<err>) | <path>`.
///
/// The `reachable` arm is why this cannot reuse the generic path
/// parser: CC prints the word instead of the path when the socket
/// answers, and treating it as a path produced a `control_sock` of
/// literally `reachable` on every healthy daemon.
fn parse_control_sock(value: &str) -> Option<PathBuf> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("reachable") {
        return None;
    }
    if v.to_ascii_lowercase().starts_with("unreachable") {
        // Try to recover the sock path from the parenthesized hint:
        //   "unreachable (connect ENOENT /tmp/.../control.sock)"
        if let (Some(open), Some(close)) = (v.find('('), v.rfind(')')) {
            if close > open {
                let inner = &v[open + 1..close];
                // Pull the last whitespace-separated token that looks
                // like an absolute path. `is_absolute_path_str` (not a
                // starts_with('/') check — .claude/rules/paths.md)
                // covers Unix, drive-letter, UNC, and named-pipe
                // (`\\.\pipe\...`) shapes.
                if let Some(last) = inner
                    .split_whitespace()
                    .rev()
                    .find(|tok| crate::path_utils::is_absolute_path_str(tok))
                {
                    return Some(PathBuf::from(last));
                }
            }
        }
        return None;
    }
    parse_absolute_path_or_none(v)
}

/// `bg workers:` has two shapes, and which number comes first differs
/// between them:
///
/// ```text
/// bg workers:   2 running (control.sock), 3 in roster.json   # control reachable
/// bg workers:   3 in roster.json (control unreachable)       # control down
/// ```
///
/// Taking the first digit run is correct for both, and deliberately so:
/// it yields the live count when the control socket can supply one, and
/// falls back to the roster count when it cannot. `bg_workers` is the
/// load-bearing field (it drives the sidebar badge and the rotation
/// audit's "N bg workers active" suffix), so both shapes are pinned by
/// fixtures rather than left to luck.
fn parse_worker_count(value: &str) -> Option<u32> {
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

    // ── sock-path recovery shapes (.claude/rules/paths.md) ──────
    // Pure string ops — run on every host OS. Verbatim `\\?\` is not
    // covered: the hint text is daemon-written, never canonicalize
    // output.

    #[test]
    fn test_daemon_sock_recovery_handles_unix_hint() {
        let v = "unreachable (connect ENOENT /tmp/cc-daemon-501/abc/control.sock)";
        assert_eq!(
            parse_control_sock(v),
            Some(PathBuf::from("/tmp/cc-daemon-501/abc/control.sock"))
        );
    }

    #[test]
    fn test_daemon_sock_recovery_handles_windows_drive_hint() {
        let v =
            r"unreachable (connect ENOENT C:\Users\j\AppData\Local\Temp\cc-daemon\control.sock)";
        assert_eq!(
            parse_control_sock(v),
            Some(PathBuf::from(
                r"C:\Users\j\AppData\Local\Temp\cc-daemon\control.sock"
            ))
        );
    }

    #[test]
    fn test_daemon_sock_recovery_handles_named_pipe_hint() {
        let v = r"unreachable (connect ENOENT \\.\pipe\cc-daemon-control)";
        assert_eq!(
            parse_control_sock(v),
            Some(PathBuf::from(r"\\.\pipe\cc-daemon-control"))
        );
    }

    #[test]
    fn test_daemon_sock_recovery_returns_none_without_path_token() {
        assert_eq!(parse_control_sock("unreachable (connect refused)"), None);
        assert_eq!(parse_control_sock("unreachable"), None);
    }

    // ── fixtures ────────────────────────────────────────────────
    // Transcribed from CC 2.1.241's `formatBgDaemonStatus`, not
    // invented. The pair below is the reason: `IDLE_FIXTURE` is the
    // shape that shipped, `IDLE_FIXTURE_LEGACY` the shape this parser
    // was written against, and only the second one has paths on the
    // roster/log lines.

    /// Live output, copied verbatim from `claude daemon status` on
    /// CC 2.1.241 (2026-08-24) with the daemon idle.
    const IDLE_FIXTURE: &str = "\
not running

bg sessions:
  sock dir:     /tmp/cc-daemon-501/5efc884f
  control.sock: unreachable (connect ENOENT /tmp/cc-daemon-501/5efc884f/control.sock)
  bg workers:   0 in roster.json (control unreachable)
  roster.json:  updated 765423s ago
  daemon.log:   18.3KB at /Users/joker/.claude/daemon.log
";

    /// The older shape, where both lines carried a bare path. Kept so
    /// the compatibility arm is exercised rather than assumed.
    const IDLE_FIXTURE_LEGACY: &str = "\
not running

bg sessions:
  sock dir:     /tmp/cc-daemon-501/5efc884f
  control.sock: unreachable (connect ENOENT /tmp/cc-daemon-501/5efc884f/control.sock)
  bg workers:   0 in roster.json (control unreachable)
  roster.json:  absent
  daemon.log:   absent
";

    /// Running daemon: header block first (leading with `pid:`, NOT a
    /// "running" line), then the bg-sessions block with the control
    /// socket up — which is the shape that prints `reachable` instead
    /// of a path and two worker numbers instead of one.
    const RUNNING_FIXTURE: &str = "\
pid:     12345
version: 2.1.241
uptime:  3600s
origin:  transient
config:  /Users/me/.claude/daemon/daemon.json
log:     /Users/me/.claude/daemon.log

bg sessions:
  sock dir:     /tmp/cc-daemon-501/abc
  control.sock: reachable
  bg workers:   2 running (control.sock), 3 in roster.json
  roster.json:  updated 12s ago
  daemon.log:   1.2MB at /Users/me/.claude/daemon.log
";

    // ── the reported defect: prose must never become a path ──────

    #[test]
    fn roster_age_line_does_not_become_a_path() {
        // Regression for #84. `updated 765423s ago` is a sentence, and
        // the parser used to hand it to `PathBuf::from` verbatim.
        let s = parse_status_output(IDLE_FIXTURE);
        assert_eq!(s.roster_path, None);
    }

    #[test]
    fn log_line_yields_the_path_not_the_size_prefix() {
        let s = parse_status_output(IDLE_FIXTURE);
        assert_eq!(
            s.log_path.as_deref(),
            Some(std::path::Path::new("/Users/joker/.claude/daemon.log")),
            "the size prefix must be stripped, not carried into the path"
        );
    }

    #[test]
    fn reachable_control_sock_does_not_become_a_path() {
        // The second instance of the same defect, in the same
        // function: CC prints the word `reachable` where it used to
        // print the socket path, so every healthy daemon reported a
        // `control_sock` of literally "reachable".
        let s = parse_status_output(RUNNING_FIXTURE);
        assert_eq!(s.control_sock, None);
    }

    #[test]
    fn running_daemon_leads_with_pid_and_reports_uptime() {
        let s = parse_status_output(RUNNING_FIXTURE);
        assert!(s.running, "a `pid:` first line means the daemon is up");
        assert_eq!(s.pid, Some(12345));
        assert_eq!(
            s.uptime_secs,
            Some(3600),
            "uptime is its own line, not part of the first one"
        );
    }

    #[test]
    fn reachable_control_reports_the_live_worker_count() {
        // Two numbers on the line; the live one comes first and is the
        // one that answers "how many workers are active".
        let s = parse_status_output(RUNNING_FIXTURE);
        assert_eq!(s.bg_workers, Some(2));
        assert!(matches!(s.parse_status, DaemonParseStatus::Ok));
    }

    #[test]
    fn legacy_path_shapes_still_parse() {
        let fixture = IDLE_FIXTURE_LEGACY
            .replace(
                "roster.json:  absent",
                "roster.json:  /Users/me/.claude/roster.json",
            )
            .replace(
                "daemon.log:   absent",
                "daemon.log:   /Users/me/.claude/daemon.log",
            );
        let s = parse_status_output(&fixture);
        assert_eq!(
            s.roster_path.as_deref(),
            Some(std::path::Path::new("/Users/me/.claude/roster.json"))
        );
        assert_eq!(
            s.log_path.as_deref(),
            Some(std::path::Path::new("/Users/me/.claude/daemon.log"))
        );
    }

    #[test]
    fn absent_stays_none_on_both_lines() {
        let s = parse_status_output(IDLE_FIXTURE_LEGACY);
        assert_eq!(s.roster_path, None);
        assert_eq!(s.log_path, None);
    }

    #[test]
    fn optional_lines_do_not_derail_the_parse() {
        // `bg sessions:  disabled`, the version-skew continuation (no
        // colon at all) and the trailing `warning:` line are all
        // things CC emits that this parser must step over without
        // corrupting a field.
        let fixture = "\
not running

bg sessions:
  sock dir:     /tmp/cc-daemon-501/abc
  control.sock: unreachable (connect ENOENT /tmp/cc-daemon-501/abc/control.sock)
  bg sessions:  disabled (start failure — see daemon.log; restart the service after fixing)
  bg workers:   1 running (control.sock), 4 in roster.json
                2 from a different CLI version (most stay attachable)
  roster.json:  updated 3s ago
  daemon.log:   18.3KB at /Users/me/.claude/daemon.log
  warning:      supervisor not running but 4 workers in roster
";
        let s = parse_status_output(fixture);
        assert!(!s.running);
        assert_eq!(s.bg_workers, Some(1));
        assert_eq!(s.roster_path, None);
        assert_eq!(
            s.log_path.as_deref(),
            Some(std::path::Path::new("/Users/me/.claude/daemon.log"))
        );
        assert!(matches!(s.parse_status, DaemonParseStatus::Ok));
    }

    #[test]
    fn windows_shapes_survive_both_lines() {
        // .claude/rules/paths.md — the classifier is string-shape
        // based, so drive-letter and UNC forms are recognized on every
        // host, not just Windows.
        assert_eq!(
            parse_log_value(r"18.3KB at C:\Users\j\.claude\daemon.log"),
            Some(PathBuf::from(r"C:\Users\j\.claude\daemon.log"))
        );
        assert_eq!(
            parse_log_value(r"18.3KB at \\server\share\daemon.log"),
            Some(PathBuf::from(r"\\server\share\daemon.log"))
        );
        assert_eq!(
            parse_roster_value(r"C:\Users\j\.claude\roster.json"),
            Some(PathBuf::from(r"C:\Users\j\.claude\roster.json"))
        );
    }

    #[test]
    fn unrecognized_prose_never_yields_a_path() {
        // The general guard, so a future CC sentence fails to None
        // rather than fabricating a path nobody can open.
        for prose in [
            "absent",
            "reachable",
            "updated 42s ago",
            "unknown",
            "",
            "some future phrasing",
        ] {
            assert_eq!(parse_roster_value(prose), None, "roster: {prose:?}");
            assert_eq!(parse_absolute_path_or_none(prose), None, "guard: {prose:?}");
        }
    }

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
        assert_eq!(s.roster_path, None, "the age line advertises no path");
        assert_eq!(
            s.log_path.as_deref(),
            Some(std::path::Path::new("/Users/joker/.claude/daemon.log"))
        );
        assert!(matches!(s.parse_status, DaemonParseStatus::Ok));
    }

    #[test]
    fn running_with_workers_parses_count() {
        // NOT CC's format — see RUNNING_FIXTURE for that. This pins the
        // defensive one-line fallback in `parse_running_line`, which is
        // what would catch a future CC that announces itself on a
        // single line again. Believing this fixture WAS the format is
        // how the real running shape went unmodelled.
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
        assert!(matches!(s.parse_status, DaemonParseStatus::Degraded { .. }));
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
    fn log_value_returns_none_for_absent() {
        assert_eq!(parse_log_value("absent"), None);
        assert_eq!(parse_log_value("Absent"), None);
        assert_eq!(
            parse_log_value("/Users/me/.claude/daemon.log"),
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
        // Print the path fields too: #84 was a *garbage value*, not a
        // crash, so a live run that only prints the load-bearing pair
        // shows green over exactly the defect being checked.
        eprintln!(
            "live: sock_dir={:?} control_sock={:?} roster_path={:?} log_path={:?}",
            s.sock_dir, s.control_sock, s.roster_path, s.log_path
        );
        for (name, p) in [
            ("sock_dir", &s.sock_dir),
            ("control_sock", &s.control_sock),
            ("roster_path", &s.roster_path),
            ("log_path", &s.log_path),
        ] {
            if let Some(p) = p {
                assert!(
                    crate::path_utils::is_absolute_path_str(&p.to_string_lossy()),
                    "{name} is not path-shaped: {p:?}"
                );
            }
        }
        // No hard assert on `running` — we don't know whether the
        // user has a daemon up. Assert only that the parser didn't
        // outright fail.
        assert!(!matches!(s.parse_status, DaemonParseStatus::Failed { .. }));
    }
}
