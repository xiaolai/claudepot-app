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
//! 1. tools/list returns all thirteen Claudepot tools with schemas.
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
    "claudepot_archive_memory",
    "claudepot_log_decision",
    "claudepot_archive_decision",
    "claudepot_submit_evidence",
    "claudepot_list_evidence",
    "claudepot_memory_links",
    "claudepot_list_memories",
    "claudepot_list_decisions",
    "claudepot_list_sessions",
    "claudepot_list_projects",
];

fn bin_path() -> PathBuf {
    // `cargo test` for an integration test typically sets
    // CARGO_BIN_EXE_<name>, but it isn't guaranteed across all
    // versions / configurations. Fall back to the target dir
    // discovered via CARGO_MANIFEST_DIR.
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_claudepot") {
        return PathBuf::from(p);
    }
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let mut p = PathBuf::from(manifest);
    p.pop(); // crates/claudepot-cli -> crates
    p.pop(); // -> workspace root
    let suffix = if cfg!(windows) {
        "claudepot.exe"
    } else {
        "claudepot"
    };
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
    // Unconfined: these tests exercise the tool surface against a
    // synthetic multi-project fixture DB, so they need to see all of
    // it. Real registrations default to confinement — the boundary
    // itself is covered by `a_confined_server_refuses_another_project`
    // and by `shared_memory::scope`'s unit tests.
    run_mcp_session_scoped(db, &["--all-projects"], frames, expected_ids)
}

fn run_mcp_session_scoped(
    db: &std::path::Path,
    scope_args: &[&str],
    frames: &str,
    expected_ids: &[u32],
) -> String {
    let mut child = Command::new(bin_path())
        .args(["mcp", "memory-server", "--db"])
        .arg(db)
        .args(scope_args)
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
fn tools_list_returns_all_thirteen_claudepot_tools() {
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

/// The confinement boundary, end to end.
///
/// `sessions.db` is a cross-project index: on a real machine it holds
/// every project the user has ever opened, including work that has
/// nothing to do with the repo the agent is currently in. Before the
/// `shared_memory::scope` boundary existed, a memory server attached
/// to project A could search project B's transcripts, read any
/// indexed file, and enumerate every project by name — and the
/// instruction snippet Claudepot ships actively told it to search
/// without a scope.
///
/// This test stages two projects, confines the server to one, and
/// asserts the other is unreachable through every read tool. It is a
/// security regression test: if it ever goes green-by-accident
/// (e.g. because a filter silently became a substring match), the
/// leak is back.
#[test]
fn a_confined_server_refuses_another_project() {
    let tmp = TempDir::new().unwrap();
    let codex_home = tmp.path().join("codex");
    let sessions = codex_home.join("sessions").join("2026/05/15");
    std::fs::create_dir_all(&sessions).unwrap();

    // Project A — the one the agent is working in.
    std::fs::write(
        sessions.join("ours.jsonl"),
        r#"{"timestamp":"2026-05-15T11:30:00.000Z","type":"session_meta","payload":{"id":"ours-1","cwd":"/work/app","originator":"codex_cli","cli_version":"0.44.0"}}
{"timestamp":"2026-05-15T11:30:00.200Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"refactor the rate limiter"}]}}
"#,
    )
    .unwrap();

    // Project B — unrelated, private. Stands in for the real case:
    // an index that also holds the user's finances or client work.
    std::fs::write(
        sessions.join("theirs.jsonl"),
        r#"{"timestamp":"2026-05-15T12:00:00.000Z","type":"session_meta","payload":{"id":"theirs-1","cwd":"/private/ledger","originator":"codex_cli","cli_version":"0.44.0"}}
{"timestamp":"2026-05-15T12:00:00.200Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"reconcile the rate limiter payment ledger"}]}}
"#,
    )
    .unwrap();

    let db = tmp.path().join("sessions.db");
    let out = Command::new(bin_path())
        .args(["codex", "index", "--codex-home"])
        .arg(&codex_home)
        .args(["--db"])
        .arg(&db)
        .arg("--json")
        .output()
        .expect("spawn claudepot codex index");
    assert!(out.status.success(), "index failed: {out:?}");

    let confined = ["--project", "/work/app"];

    // 1. Search reaches our project...
    let frames = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{},\"clientInfo\":{\"name\":\"e2e\",\"version\":\"0\"}}}\n\
                  {\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n\
                  {\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"claudepot_search_memory\",\"arguments\":{\"query\":\"rate limiter\"}}}\n";
    let stdout = run_mcp_session_scoped(&db, &confined, frames, &[1, 2]);
    assert!(
        stdout.contains("ours-1"),
        "a confined server must still see its OWN project:\n{stdout}"
    );

    // ...and the same query does NOT surface the other project, even
    // though its transcript matches the search terms.
    assert!(
        !stdout.contains("theirs-1") && !stdout.contains("ledger"),
        "LEAK: another project's transcript surfaced in a confined search:\n{stdout}"
    );

    // 2. Reading the other project's transcript by locator is denied.
    //    (An agent could otherwise guess or be handed a file_path.)
    let theirs = sessions.join("theirs.jsonl");
    let theirs_json = serde_json::to_string(&theirs.to_string_lossy()).unwrap();
    let frames = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{{}},\"clientInfo\":{{\"name\":\"e2e\",\"version\":\"0\"}}}}}}\n\
         {{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}}\n\
         {{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{{\"name\":\"claudepot_read_conversation\",\"arguments\":{{\"file_path\":{theirs_json}}}}}}}\n"
    );
    let stdout = run_mcp_session_scoped(&db, &confined, &frames, &[1, 2]);
    // The read must be REFUSED and must not return the other project's
    // content. Two refusal codes are both correct here: `scope_denied`
    // (the confinement check fired) or `locator_not_indexed` (the exact
    // file_path didn't resolve — e.g. on Windows the caller-built path
    // separators differ from the stored form). Either way no bytes cross;
    // the security property is "the content did not leak", asserted next.
    // The scope check itself is exercised precisely, on every platform,
    // by the `shared_memory::scope` unit tests.
    assert!(
        stdout.contains("scope_denied") || stdout.contains("locator_not_indexed"),
        "reading another project's transcript must be refused:\n{stdout}"
    );
    assert!(
        !stdout.contains("payment ledger"),
        "LEAK: denied read still returned the other project's content:\n{stdout}"
    );

    // 3. Listing projects shows ours and only ours — a directory name
    //    is itself disclosure (clients, subjects, private work).
    let frames = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{},\"clientInfo\":{\"name\":\"e2e\",\"version\":\"0\"}}}\n\
                  {\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n\
                  {\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"claudepot_list_projects\",\"arguments\":{}}}\n";
    let stdout = run_mcp_session_scoped(&db, &confined, frames, &[1, 2]);
    assert!(
        stdout.contains("/work/app"),
        "confined list_projects must still show our own project:\n{stdout}"
    );
    assert!(
        !stdout.contains("/private/ledger"),
        "LEAK: another project was enumerated by name:\n{stdout}"
    );

    // 4. Asking for the other project by name is refused outright,
    //    not silently answered with our own rows.
    let frames = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{},\"clientInfo\":{\"name\":\"e2e\",\"version\":\"0\"}}}\n\
                  {\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n\
                  {\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"claudepot_search_memory\",\"arguments\":{\"query\":\"ledger\",\"project_path\":\"/private/ledger\"}}}\n";
    let stdout = run_mcp_session_scoped(&db, &confined, frames, &[1, 2]);
    assert!(
        stdout.contains("scope_denied"),
        "an explicit request for another project must be refused:\n{stdout}"
    );
}

// ─── the knowledge-compiler surface (v5) ─────────────────────────

/// The review gate, read side, end to end.
///
/// Memories and distilled lessons share one table, discriminated by
/// `review_state`. The instruction snippet tells every agent to call
/// `claudepot_list_memories` at session start and treat the result as
/// load-bearing context — so this emission is exactly the surface the
/// human review queue exists to guard. Before v5 it served unreviewed
/// proposals and explicitly-REJECTED claims as if a human had vetted
/// them.
#[test]
fn list_memories_enforces_the_review_gate() {
    use claudepot_core::session_index::SessionIndex;
    use claudepot_core::shared_memory::durable::{
        create_memory, create_proposal, CreatedByKind, MemoryKind, NewMemory, NewProposal, Scope,
    };
    use claudepot_core::shared_memory::review;

    let tmp = TempDir::new().unwrap();
    let db = tmp.path().join("sessions.db");
    {
        let idx = SessionIndex::open(&db).unwrap();
        // A human-vetted memory (create_memory rows land 'accepted').
        create_memory(
            &idx,
            &NewMemory {
                scope: Scope::Project,
                project_path: Some("/work/app"),
                kind: MemoryKind::Fact,
                content: "the human vetted this fact",
                created_by_kind: CreatedByKind::User,
                created_by: "user:test",
                confidence: None,
            },
        )
        .unwrap();
        // An unreviewed distiller proposal.
        create_proposal(
            &idx,
            &NewProposal {
                project_path: "/work/app",
                kind: MemoryKind::Constraint,
                content: "unreviewed distiller claim",
                directive: "do the unreviewed thing",
                confidence: 80,
                anchor_json: None,
                origin_exchange_id: None,
                origin_file_path: None,
                created_by: "agent:distiller",
            },
        )
        .unwrap();
        // A claim the human explicitly rejected.
        let rejected = create_proposal(
            &idx,
            &NewProposal {
                project_path: "/work/app",
                kind: MemoryKind::Constraint,
                content: "the human said no to this",
                directive: "never do this",
                confidence: 80,
                anchor_json: None,
                origin_exchange_id: None,
                origin_file_path: None,
                created_by: "agent:distiller",
            },
        )
        .unwrap();
        assert!(review::reject(&idx, &rejected.id, 1_000).unwrap());
    }

    let frames = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{},\"clientInfo\":{\"name\":\"it\",\"version\":\"0\"}}}\n\
                  {\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n\
                  {\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"claudepot_list_memories\",\"arguments\":{}}}\n";
    let stdout = run_mcp_session(&db, frames, &[1, 2]);
    assert!(
        stdout.contains("the human vetted this fact"),
        "the accepted memory must surface:\n{stdout}"
    );
    assert!(
        !stdout.contains("unreviewed distiller claim"),
        "REVIEW-GATE BYPASS: an unreviewed proposal crossed the MCP boundary:\n{stdout}"
    );
    assert!(
        !stdout.contains("the human said no to this"),
        "REVIEW-GATE BYPASS: a rejected claim crossed the MCP boundary:\n{stdout}"
    );
    assert!(
        stdout.contains("review_state"),
        "rows must carry review_state so agents can discriminate:\n{stdout}"
    );
}

/// Evidence round-trip plus the v5 write auto-fill, on a CONFINED
/// server. Before v5, an omitted project_path silently produced a
/// *global* row — which the confined list (pinned to the root) could
/// then never read back.
///
/// Writes and reads run in SEPARATE sessions: rmcp serves tool calls
/// concurrently within one session, so a write→read ordering inside a
/// single session is not guaranteed.
#[test]
fn evidence_round_trips_on_a_confined_server_without_project_path() {
    let tmp = TempDir::new().unwrap();
    let db = tmp.path().join("sessions.db");
    let confined = ["--project", "/work/app"];

    // Session A: the writes, both with project_path OMITTED.
    let frames = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{},\"clientInfo\":{\"name\":\"it\",\"version\":\"0\"}}}\n\
                  {\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n\
                  {\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"claudepot_submit_evidence\",\"arguments\":{\"summary\":\"fixed the flaky retry loop\",\"verification\":\"cargo test green\",\"files_changed\":\"[\\\"src/retry.rs\\\"]\",\"confidence\":90,\"created_by\":\"it:test\"}}}\n\
                  {\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"claudepot_log_decision\",\"arguments\":{\"decision\":\"retries use exponential backoff\",\"created_by\":\"it:test\"}}}\n";
    let _ = run_mcp_session_scoped(&db, &confined, frames, &[1, 2, 3]);

    // Session B: the confined reads see both rows — proof the writes
    // landed on THIS project, not as unreadable global rows.
    let frames = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{},\"clientInfo\":{\"name\":\"it\",\"version\":\"0\"}}}\n\
                  {\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n\
                  {\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"claudepot_list_evidence\",\"arguments\":{}}}\n\
                  {\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{\"name\":\"claudepot_list_decisions\",\"arguments\":{}}}\n";
    let stdout = run_mcp_session_scoped(&db, &confined, frames, &[1, 2, 3]);
    assert!(
        stdout.contains("fixed the flaky retry loop"),
        "list_evidence must read back the just-submitted evidence (auto-filled to this project):\n{stdout}"
    );
    assert!(
        stdout.contains("retries use exponential backoff"),
        "list_decisions must read back the just-logged decision (auto-filled to this project):\n{stdout}"
    );

    // And the confinement holds for the new read tool: asking for
    // another project's evidence by name is refused.
    let frames = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{},\"clientInfo\":{\"name\":\"it\",\"version\":\"0\"}}}\n\
                  {\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n\
                  {\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"claudepot_list_evidence\",\"arguments\":{\"project_path\":\"/private/ledger\"}}}\n";
    let stdout = run_mcp_session_scoped(&db, &confined, frames, &[1, 2]);
    assert!(
        stdout.contains("scope_denied"),
        "list_evidence for another project must be refused:\n{stdout}"
    );
}

/// A wrong memory is retractable over MCP — and a confined server
/// cannot retract another project's memory by guessing its id.
#[test]
fn archive_memory_retracts_and_respects_confinement() {
    use claudepot_core::session_index::SessionIndex;
    use claudepot_core::shared_memory::durable::{
        create_memory, list_memories, CreatedByKind, MemoryKind, MemoryListFilter, NewMemory,
        ReviewVisibility, Scope,
    };

    let tmp = TempDir::new().unwrap();
    let db = tmp.path().join("sessions.db");
    let (ours, theirs) = {
        let idx = SessionIndex::open(&db).unwrap();
        let ours = create_memory(
            &idx,
            &NewMemory {
                scope: Scope::Project,
                project_path: Some("/work/app"),
                kind: MemoryKind::Fact,
                content: "obsolete fact to retract",
                created_by_kind: CreatedByKind::Agent,
                created_by: "it:test",
                confidence: None,
            },
        )
        .unwrap();
        let theirs = create_memory(
            &idx,
            &NewMemory {
                scope: Scope::Project,
                project_path: Some("/private/ledger"),
                kind: MemoryKind::Fact,
                content: "another project's memory",
                created_by_kind: CreatedByKind::User,
                created_by: "user:test",
                confidence: None,
            },
        )
        .unwrap();
        (ours.id, theirs.id)
    };
    let confined = ["--project", "/work/app"];

    // Session A (writes; rmcp serves calls concurrently, so the read
    // that must observe the archive runs in its own session below):
    // archiving our own memory succeeds; the other project's is
    // refused outright.
    let frames = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{{}},\"clientInfo\":{{\"name\":\"it\",\"version\":\"0\"}}}}}}\n\
         {{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}}\n\
         {{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{{\"name\":\"claudepot_archive_memory\",\"arguments\":{{\"id\":\"{ours}\"}}}}}}\n\
         {{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{{\"name\":\"claudepot_archive_memory\",\"arguments\":{{\"id\":\"{theirs}\"}}}}}}\n"
    );
    let stdout = run_mcp_session_scoped(&db, &confined, &frames, &[1, 2, 3]);
    assert!(
        stdout.contains("\\\"archived\\\":true"),
        "archiving our own memory must succeed:\n{stdout}"
    );
    assert!(
        stdout.contains("scope_denied"),
        "archiving another project's memory must be refused:\n{stdout}"
    );

    // Session B: OUR memory has left the listing (the archive took) and,
    // decisively, the OTHER project's memory is still active — the
    // cross-project archive was refused, not silently applied. Checking
    // survival directly (via core) rules out an inverted outcome that a
    // concatenated-stdout match could otherwise pass.
    let frames = "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{},\"clientInfo\":{\"name\":\"it\",\"version\":\"0\"}}}\n\
                  {\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n\
                  {\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{\"name\":\"claudepot_list_memories\",\"arguments\":{}}}\n";
    let stdout = run_mcp_session_scoped(&db, &confined, frames, &[1, 2]);
    assert!(
        !stdout.contains("obsolete fact to retract"),
        "the archived memory must leave the listing:\n{stdout}"
    );

    let idx = SessionIndex::open(&db).unwrap();
    let their_rows = list_memories(
        &idx,
        &MemoryListFilter {
            project_path: Some("/private/ledger".to_string()),
            review: ReviewVisibility::All,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(
        their_rows.len(),
        1,
        "the other project's memory must survive the refused archive"
    );
    assert!(
        their_rows[0].archived_at_ms.is_none(),
        "the other project's memory must remain active, id={theirs}"
    );
}

/// Provenance links are readable over MCP, and the parent-id
/// validation rejects an ambiguous query.
///
/// `memory_links` targets are FOREIGN KEYs into the transcript index
/// (`exchanges.id` / `sessions.file_path`), so the fixture stages a
/// real Codex rollout and indexes it before linking — the same path
/// production links take.
#[test]
fn memory_links_read_back_provenance() {
    use claudepot_core::session_index::SessionIndex;
    use claudepot_core::shared_memory::durable::{
        create_memory, link, CreatedByKind, LinkParent, LinkRelation, LinkTarget, MemoryKind,
        NewLink, NewMemory, Scope,
    };
    use claudepot_core::shared_memory::search as sms;

    let tmp = TempDir::new().unwrap();
    let codex_home = tmp.path().join("codex");
    let sessions = codex_home.join("sessions").join("2026/05/15");
    std::fs::create_dir_all(&sessions).unwrap();
    std::fs::write(
        sessions.join("origin.jsonl"),
        r#"{"timestamp":"2026-05-15T11:30:00.000Z","type":"session_meta","payload":{"id":"origin-1","cwd":"/work/app","originator":"codex_cli","cli_version":"0.44.0"}}
{"timestamp":"2026-05-15T11:30:00.200Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"the lesson was learned here"}]}}
"#,
    )
    .unwrap();

    let db = tmp.path().join("sessions.db");
    let out = Command::new(bin_path())
        .args(["codex", "index", "--codex-home"])
        .arg(&codex_home)
        .args(["--db"])
        .arg(&db)
        .arg("--json")
        .output()
        .expect("spawn claudepot codex index");
    assert!(out.status.success(), "index failed: {out:?}");

    let memory_id = {
        let idx = SessionIndex::open(&db).unwrap();
        // Link to the session's file_path AS STORED by the indexer —
        // guessing the path would fight canonicalization.
        let indexed = sms::list_sessions(&idx, &sms::SessionListFilter::default()).unwrap();
        assert_eq!(indexed.len(), 1, "fixture must index exactly one session");
        let file_path = indexed[0].file_path.clone();

        let m = create_memory(
            &idx,
            &NewMemory {
                scope: Scope::Project,
                project_path: Some("/work/app"),
                kind: MemoryKind::Pattern,
                content: "learned from a transcript",
                created_by_kind: CreatedByKind::Agent,
                created_by: "it:test",
                confidence: None,
            },
        )
        .unwrap();
        link(
            &idx,
            &NewLink {
                parent: LinkParent::Memory(&m.id),
                target: LinkTarget::File(&file_path),
                relation: LinkRelation::Origin,
            },
        )
        .unwrap();
        m.id
    };

    let frames = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{{}},\"clientInfo\":{{\"name\":\"it\",\"version\":\"0\"}}}}}}\n\
         {{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}}\n\
         {{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{{\"name\":\"claudepot_memory_links\",\"arguments\":{{\"memory_id\":\"{memory_id}\"}}}}}}\n\
         {{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"tools/call\",\"params\":{{\"name\":\"claudepot_memory_links\",\"arguments\":{{}}}}}}\n"
    );
    let stdout = run_mcp_session(&db, &frames, &[1, 2, 3]);
    assert!(
        stdout.contains("origin.jsonl") && stdout.contains("\\\"relation\\\":\\\"origin\\\""),
        "the origin link must read back with its file locator and relation:\n{stdout}"
    );
    assert!(
        stdout.contains("invalid_link_query"),
        "an ambiguous links query (no parent id) must be rejected:\n{stdout}"
    );
}

/// `memory_links` honors confinement on the link TARGET, not just the
/// parent. A GLOBAL parent skips the parent-scope check (global rows are
/// visible to every server), so its links — which can point into any
/// project — are the leak vector. A confined server must drop the ones
/// whose transcript belongs to another project.
#[test]
fn memory_links_filters_cross_project_link_targets() {
    use claudepot_core::session_index::SessionIndex;
    use claudepot_core::shared_memory::durable::{
        create_memory, link, CreatedByKind, LinkParent, LinkRelation, LinkTarget, MemoryKind,
        NewLink, NewMemory, Scope,
    };
    use claudepot_core::shared_memory::search as sms;

    // Two projects, each with an indexed transcript.
    let tmp = TempDir::new().unwrap();
    let codex_home = tmp.path().join("codex");
    let sessions = codex_home.join("sessions").join("2026/05/15");
    std::fs::create_dir_all(&sessions).unwrap();
    std::fs::write(
        sessions.join("ours.jsonl"),
        r#"{"timestamp":"2026-05-15T11:30:00.000Z","type":"session_meta","payload":{"id":"ours-1","cwd":"/work/app","originator":"codex_cli","cli_version":"0.44.0"}}
{"timestamp":"2026-05-15T11:30:00.200Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"our lesson"}]}}
"#,
    )
    .unwrap();
    std::fs::write(
        sessions.join("theirs.jsonl"),
        r#"{"timestamp":"2026-05-15T12:00:00.000Z","type":"session_meta","payload":{"id":"theirs-1","cwd":"/private/ledger","originator":"codex_cli","cli_version":"0.44.0"}}
{"timestamp":"2026-05-15T12:00:00.200Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"their private ledger note"}]}}
"#,
    )
    .unwrap();

    let db = tmp.path().join("sessions.db");
    let out = Command::new(bin_path())
        .args(["codex", "index", "--codex-home"])
        .arg(&codex_home)
        .args(["--db"])
        .arg(&db)
        .arg("--json")
        .output()
        .expect("spawn claudepot codex index");
    assert!(out.status.success(), "index failed: {out:?}");

    // A GLOBAL memory linked to BOTH projects' transcripts.
    let global_id = {
        let idx = SessionIndex::open(&db).unwrap();
        let all = sms::list_sessions(&idx, &sms::SessionListFilter::default()).unwrap();
        let fp = |proj: &str| {
            all.iter()
                .find(|s| s.project_path == proj)
                .unwrap()
                .file_path
                .clone()
        };
        let ours_fp = fp("/work/app");
        let theirs_fp = fp("/private/ledger");

        let m = create_memory(
            &idx,
            &NewMemory {
                scope: Scope::Global,
                project_path: None,
                kind: MemoryKind::Fact,
                content: "a global memory linked across projects",
                created_by_kind: CreatedByKind::Agent,
                created_by: "it:test",
                confidence: None,
            },
        )
        .unwrap();
        link(
            &idx,
            &NewLink {
                parent: LinkParent::Memory(&m.id),
                target: LinkTarget::File(&ours_fp),
                relation: LinkRelation::Origin,
            },
        )
        .unwrap();
        link(
            &idx,
            &NewLink {
                parent: LinkParent::Memory(&m.id),
                target: LinkTarget::File(&theirs_fp),
                relation: LinkRelation::Related,
            },
        )
        .unwrap();
        m.id
    };

    // Confined to /work/app: querying the global parent's links returns
    // ONLY the in-scope link; the /private/ledger target is dropped.
    let confined = ["--project", "/work/app"];
    let frames = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{{}},\"clientInfo\":{{\"name\":\"it\",\"version\":\"0\"}}}}}}\n\
         {{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}}\n\
         {{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"tools/call\",\"params\":{{\"name\":\"claudepot_memory_links\",\"arguments\":{{\"memory_id\":\"{global_id}\"}}}}}}\n"
    );
    let stdout = run_mcp_session_scoped(&db, &confined, &frames, &[1, 2]);
    assert!(
        stdout.contains("ours.jsonl"),
        "the in-scope link must be returned:\n{stdout}"
    );
    assert!(
        !stdout.contains("theirs.jsonl") && !stdout.contains("ledger"),
        "LEAK: a cross-project link target surfaced through a global parent:\n{stdout}"
    );

    // Unconfined sees both — proof the filter is scope-driven, not a
    // blanket drop.
    let stdout = run_mcp_session(&db, &frames, &[1, 2]);
    assert!(
        stdout.contains("ours.jsonl") && stdout.contains("theirs.jsonl"),
        "an --all-projects server must return every link:\n{stdout}"
    );
}
