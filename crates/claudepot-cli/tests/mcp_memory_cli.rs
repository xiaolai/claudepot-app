//! Integration test for `claudepot mcp memory-server` and the
//! `claudepot codex index → MCP search` end-to-end loop.
//!
//! Replaces fixed-sleep harnesses with a wait-on-stdout-pattern:
//! the runner reads frames until expected ids are seen, then
//! closes stdin (end-of-input signal) and waits for the child to
//! exit. No `kill -INT`, no fixed sleep races. Works equivalently
//! on macOS, Linux, and Windows.
//!
//! Test cases:
//! 1. tools/list returns all eight Claudepot tools with schemas.
//! 2. claudepot_remember + claudepot_list_memories round-trip
//!    across a server restart.
//! 3. Stdout is JSON-RPC only — no logging pollution.
//! 4. End-to-end: stage Codex rollouts, run `claudepot codex index`
//!    via the CLI, then via MCP call claudepot_search_memory and
//!    assert a hit lands. This covers H4's plan-to-production gap.

use std::collections::HashSet;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tempfile::TempDir;

const TOOLS: &[&str] = &[
    "claudepot_search_memory",
    "claudepot_read_conversation",
    "claudepot_remember",
    "claudepot_log_decision",
    "claudepot_archive_decision",
    "claudepot_submit_evidence",
    "claudepot_list_memories",
    "claudepot_list_decisions",
];

fn bin_path() -> PathBuf {
    // `cargo test` for an integration test typically sets
    // CARGO_BIN_EXE_<name>, but it isn't guaranteed across all
    // versions / configurations. Fall back to the target dir
    // discovered via CARGO_MANIFEST_DIR.
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_claudepot") {
        return PathBuf::from(p);
    }
    let manifest = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR not set");
    let mut p = PathBuf::from(manifest);
    p.pop(); // crates/claudepot-cli -> crates
    p.pop(); // -> workspace root
    let suffix = if cfg!(windows) { "claudepot.exe" } else { "claudepot" };
    p.push("target");
    p.push("debug");
    p.push(suffix);
    assert!(
        p.exists(),
        "binary not found at {} — run `cargo build -p claudepot-cli` first \
         (CI must build the binary before `cargo test` runs)",
        p.display()
    );
    p
}

/// Drive one MCP stdio session. `frames` is the newline-delimited
/// JSON-RPC input. `expected_ids` is the set of `id`s the harness
/// waits for before closing stdin and reaping the child. Returns
/// the concatenated stdout (every line guaranteed to be a valid
/// JSON-RPC envelope by the server's stdout discipline).
///
/// No fixed sleeps. The harness reads with a per-frame deadline of
/// 8 seconds (generous for cold-start CI); a timeout means the
/// test fails with diagnostic context instead of hanging.
fn run_mcp_session(db: &std::path::Path, frames: &str, expected_ids: &[u32]) -> String {
    let mut child = Command::new(bin_path())
        .args(["mcp", "memory-server", "--db"])
        .arg(db)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        // Don't override RUST_LOG — the M9 stderr-pinned
        // subscriber should keep stderr from polluting stdout
        // regardless of log level. Verifying this is the whole
        // point of test #3.
        .spawn()
        .expect("spawn claudepot mcp memory-server");

    let mut stdin = child.stdin.take().expect("stdin");
    stdin
        .write_all(frames.as_bytes())
        .expect("write frames to mcp stdin");
    // Don't drop stdin yet — closing it would tell rmcp to exit
    // before it has a chance to respond. We hold stdin open until
    // the expected ids have been seen on stdout.

    let stdout = child.stdout.take().expect("stdout");
    let mut reader = BufReader::new(stdout);
    let mut collected = String::new();
    let mut seen: HashSet<u32> = HashSet::new();
    let deadline = Instant::now() + Duration::from_secs(8);
    let want: HashSet<u32> = expected_ids.iter().copied().collect();

    while seen != want && Instant::now() < deadline {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => break, // EOF
            Ok(_) => {
                if let Some(id) = parse_response_id(&line) {
                    seen.insert(id);
                }
                collected.push_str(&line);
            }
            Err(_) => break,
        }
    }

    // Close stdin → rmcp stops reading, the service loop exits,
    // the process terminates cleanly. No SIGKILL, no buffer loss.
    drop(stdin);

    // Drain any final frames the server emits as it shuts down.
    let drain_deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < drain_deadline {
        let mut line = String::new();
        match reader.read_line(&mut line) {
            Ok(0) => break,
            Ok(_) => collected.push_str(&line),
            Err(_) => break,
        }
    }

    let _ = child.wait();
    if seen != want {
        let missing: Vec<u32> = want.difference(&seen).copied().collect();
        panic!(
            "MCP session timed out waiting for ids {missing:?}\n--- stdout so far ---\n{collected}"
        );
    }
    collected
}

/// Extract the JSON-RPC id (if present) from a stdout line.
/// Returns `None` for notification frames and parse errors.
fn parse_response_id(line: &str) -> Option<u32> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let v: serde_json::Value = serde_json::from_str(trimmed).ok()?;
    v.get("id").and_then(|i| i.as_u64()).map(|n| n as u32)
}

#[test]
fn tools_list_returns_all_eight_claudepot_tools() {
    let tmp = TempDir::new().unwrap();
    let db = tmp.path().join("sessions.db");
    let frames = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{},\"clientInfo\":{\"name\":\"it\",\"version\":\"0\"}}}\n\
                  {\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n\
                  {\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\"}\n";
    let stdout = run_mcp_session(&db, frames, &[1, 2]);
    for t in TOOLS {
        assert!(
            stdout.contains(t),
            "tools/list missing {t}\nstdout:\n{stdout}"
        );
    }
}

#[test]
fn remember_then_list_round_trips_across_restart() {
    let tmp = TempDir::new().unwrap();
    let db = tmp.path().join("sessions.db");

    // Session A: initialize + remember. Wait for id 2 (the remember
    // response) before closing stdin so we know the write committed.
    let frames_a = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{},\"clientInfo\":{\"name\":\"it\",\"version\":\"0\"}}}\n\
                    {\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n\
                    {\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"claudepot_remember\",\"arguments\":{\"scope\":\"global\",\"kind\":\"fact\",\"content\":\"persistence works\",\"created_by\":\"it:test\"}}}\n";
    let _ = run_mcp_session(&db, frames_a, &[1, 2]);

    // Session B: list and check.
    let frames_b = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{},\"clientInfo\":{\"name\":\"it\",\"version\":\"0\"}}}\n\
                    {\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n\
                    {\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"claudepot_list_memories\",\"arguments\":{}}}\n";
    let stdout = run_mcp_session(&db, frames_b, &[1, 2]);
    assert!(
        stdout.contains("persistence works"),
        "list_memories did not see the persisted row:\n{stdout}"
    );
}

#[test]
fn stdout_only_emits_jsonrpc_frames_at_default_log_level() {
    // H1 verification — default log level (no RUST_LOG override).
    // Every stdout line must be a JSON-RPC envelope; no tracing
    // output should leak through.
    let tmp = TempDir::new().unwrap();
    let db = tmp.path().join("sessions.db");
    let frames = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{},\"clientInfo\":{\"name\":\"it\",\"version\":\"0\"}}}\n\
                  {\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n\
                  {\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/list\"}\n";
    let stdout = run_mcp_session(&db, frames, &[1, 2]);
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(trimmed);
        assert!(
            parsed.is_ok(),
            "stdout pollution — non-JSON line: {trimmed:?}\nFull stdout:\n{stdout}"
        );
    }
}

#[test]
fn mcp_returns_categorized_error_envelopes_for_invalid_input() {
    // Codex audit L-testing — exercise the MCP error envelope
    // paths so a regression that loses error_code or breaks
    // structured-error categorization gets caught.
    let tmp = TempDir::new().unwrap();
    let db = tmp.path().join("sessions.db");

    // Invalid scope on claudepot_remember → error_code = "invalid_scope".
    // Invalid kind on claudepot_remember → error_code = "invalid_kind".
    // Unknown file_path on claudepot_read_conversation → error_code = "locator_not_indexed".
    let frames = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{},\"clientInfo\":{\"name\":\"it\",\"version\":\"0\"}}}\n\
                  {\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n\
                  {\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"claudepot_remember\",\"arguments\":{\"scope\":\"bogus\",\"kind\":\"fact\",\"content\":\"x\"}}}\n\
                  {\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"claudepot_remember\",\"arguments\":{\"scope\":\"global\",\"kind\":\"bogus\",\"content\":\"x\"}}}\n\
                  {\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"tools/call\",\"params\":{\"name\":\"claudepot_read_conversation\",\"arguments\":{\"file_path\":\"/nonexistent/path.jsonl\"}}}\n";
    let stdout = run_mcp_session(&db, frames, &[1, 2, 3, 4]);
    assert!(
        stdout.contains("invalid_scope"),
        "missing 'invalid_scope' error_code in:\n{stdout}"
    );
    assert!(
        stdout.contains("invalid_kind"),
        "missing 'invalid_kind' error_code in:\n{stdout}"
    );
    assert!(
        stdout.contains("locator_not_indexed"),
        "missing 'locator_not_indexed' error_code in:\n{stdout}"
    );
    // The error envelope must include schema_version so callers
    // can branch on protocol shape.
    assert!(
        stdout.contains("\\\"schema_version\\\":1"),
        "MCP error payloads must include schema_version=1: {stdout}"
    );
}

#[test]
fn end_to_end_codex_index_then_search() {
    // H4 verification — wire the indexer to a CLI verb and prove
    // the full pipeline: stage a Codex rollout → `claudepot codex
    // index` → MCP `claudepot_search_memory` → search hit.
    let tmp = TempDir::new().unwrap();
    let codex_home = tmp.path().join("codex");
    let codex_sessions = codex_home.join("sessions").join("2026/05/15");
    std::fs::create_dir_all(&codex_sessions).unwrap();
    std::fs::write(
        codex_sessions.join("rollout.jsonl"),
        r#"{"timestamp":"2026-05-15T11:30:00.000Z","type":"session_meta","payload":{"id":"e2e-1","cwd":"/proj","originator":"codex_cli","cli_version":"0.44.0"}}
{"timestamp":"2026-05-15T11:30:00.200Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"refactor the rate limiter"}]}}
{"timestamp":"2026-05-15T11:30:02.000Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"changed the bucket from token to leaky"}]}}
"#,
    )
    .unwrap();

    let db = tmp.path().join("sessions.db");

    // 1. `claudepot codex index` populates sessions.db with the
    //    Codex rollout. --codex-home is the CODEX_HOME path; the
    //    indexer appends `sessions/` internally.
    let output = Command::new(bin_path())
        .args(["codex", "index"])
        .args(["--codex-home"])
        .arg(&codex_home)
        .args(["--db"])
        .arg(&db)
        .args(["--json"])
        .output()
        .expect("spawn claudepot codex index");
    assert!(
        output.status.success(),
        "claudepot codex index exited non-zero: {}\nstdout: {}\nstderr: {}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("valid JSON from index");
    assert_eq!(report["discovered"], 1, "report: {report}");
    assert_eq!(report["indexed"], 1, "report: {report}");

    // 2. MCP `claudepot_search_memory` finds the indexed rollout.
    let frames = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{},\"clientInfo\":{\"name\":\"e2e\",\"version\":\"0\"}}}\n\
                  {\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n\
                  {\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"claudepot_search_memory\",\"arguments\":{\"query\":\"rate limiter\"}}}\n";
    let stdout = run_mcp_session(&db, frames, &[1, 2]);
    assert!(
        stdout.contains("e2e-1") || stdout.contains("refactor the rate limiter"),
        "MCP search didn't find the indexed Codex rollout:\n{stdout}"
    );
}
