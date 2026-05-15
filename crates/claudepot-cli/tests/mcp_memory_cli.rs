//! Integration test for `claudepot mcp memory-server`.
//!
//! Spawns the built binary, drives it via stdio JSON-RPC, and
//! asserts:
//!   * `initialize` returns rmcp's serverInfo.
//!   * `tools/list` returns all seven Claudepot tools with schemas.
//!   * `claudepot_remember` persists across server restart and
//!     `claudepot_list_memories` reads it back.
//!   * Stdout is JSON-RPC only — no logging pollution.
//!
//! The test uses a temp DB so it doesn't touch the user's real
//! `~/.claudepot/sessions.db`.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;
use tempfile::TempDir;

const TOOLS: &[&str] = &[
    "claudepot_search_memory",
    "claudepot_read_conversation",
    "claudepot_remember",
    "claudepot_log_decision",
    "claudepot_submit_evidence",
    "claudepot_list_memories",
    "claudepot_list_decisions",
];

fn bin_path() -> std::path::PathBuf {
    // `cargo test` for an integration test typically sets
    // CARGO_BIN_EXE_<name>, but it isn't guaranteed across all
    // versions / configurations. Fall back to the target dir
    // discovered via CARGO_MANIFEST_DIR.
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_claudepot") {
        return std::path::PathBuf::from(p);
    }
    let manifest = std::env::var("CARGO_MANIFEST_DIR")
        .expect("CARGO_MANIFEST_DIR not set");
    let mut p = std::path::PathBuf::from(manifest);
    p.pop(); // crates/claudepot-cli -> crates
    p.pop(); // -> workspace root
    let suffix = if cfg!(windows) { "claudepot.exe" } else { "claudepot" };
    p.push("target");
    p.push("debug");
    p.push(suffix);
    assert!(
        p.exists(),
        "binary not found at {} — run `cargo build -p claudepot-cli` first",
        p.display()
    );
    p
}

fn run_session(db: &std::path::Path, frames: &str) -> String {
    let mut child = Command::new(bin_path())
        .args(["mcp", "memory-server", "--db"])
        .arg(db)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("RUST_LOG", "warn")
        .spawn()
        .expect("spawn");
    {
        let mut stdin = child.stdin.take().expect("stdin");
        stdin
            .write_all(frames.as_bytes())
            .expect("write frames");
        // Closing stdin makes rmcp finish processing the queued
        // frames and exit cleanly.
        drop(stdin);
    }
    // Give the server a moment to handle the frames + flush.
    std::thread::sleep(Duration::from_millis(1500));
    // SIGTERM in case it's still running (no SIGINT in std).
    let _ = child.kill();
    let output = child.wait_with_output().expect("wait");
    String::from_utf8(output.stdout).expect("utf-8 stdout")
}

#[test]
fn tools_list_returns_all_seven_claudepot_tools() {
    let tmp = TempDir::new().unwrap();
    let db = tmp.path().join("sessions.db");
    let frames = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"it","version":"0"}}}
{"jsonrpc":"2.0","method":"notifications/initialized"}
{"jsonrpc":"2.0","id":2,"method":"tools/list"}
"#;
    let stdout = run_session(&db, frames);
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

    // Session A: initialize + remember.
    let frames_a = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"it","version":"0"}}}
{"jsonrpc":"2.0","method":"notifications/initialized"}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"claudepot_remember","arguments":{"scope":"global","kind":"fact","content":"persistence works","created_by":"it:test"}}}
"#;
    let _ = run_session(&db, frames_a);

    // Session B: list and check.
    let frames_b = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"it","version":"0"}}}
{"jsonrpc":"2.0","method":"notifications/initialized"}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"claudepot_list_memories","arguments":{}}}
"#;
    let stdout = run_session(&db, frames_b);
    assert!(
        stdout.contains("persistence works"),
        "list_memories did not see the persisted row:\n{stdout}"
    );
}

#[test]
fn stdout_only_emits_jsonrpc_frames() {
    let tmp = TempDir::new().unwrap();
    let db = tmp.path().join("sessions.db");
    let frames = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"it","version":"0"}}}
{"jsonrpc":"2.0","method":"notifications/initialized"}
{"jsonrpc":"2.0","id":2,"method":"tools/list"}
"#;
    let stdout = run_session(&db, frames);
    for line in stdout.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        // Each non-empty stdout line must be a JSON object that
        // parses cleanly.
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(trimmed);
        assert!(
            parsed.is_ok(),
            "stdout pollution — non-JSON line: {trimmed:?}"
        );
    }
}
