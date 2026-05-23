//! Golden tests for the Phase-2 `claudepot agent draft` verb.
//!
//! Verifies the security-spine invariants:
//! - `agent draft` writes a record with `lifecycle = "draft"`.
//! - `agent draft` materializes **no** scheduler artifact and no
//!   per-agent run directory — a draft is inert on disk.
//! - both accepted input shapes work: Claudepot-native JSON and
//!   `AgentDefinition`-shaped JSON (PRD D2).
//! - `agent list` / `agent show` round-trip the new draft.
//!
//! The CLI persists the agent store under
//! `$CLAUDEPOT_DATA_DIR/agents.json`; the data dir is a fresh
//! tempdir per test so the suite is hermetic.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use tempfile::TempDir;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_claudepot")
}

struct Env {
    _data: TempDir,
    data_path: PathBuf,
}

fn make_env() -> Env {
    let data = TempDir::new().unwrap();
    let data_path = data.path().to_path_buf();
    Env {
        _data: data,
        data_path,
    }
}

/// Run `claudepot agent <args…>` with the data dir pinned to the
/// test's tempdir. Returns (stdout, stderr, exit_code).
fn run_agent(env: &Env, args: &[&str]) -> (String, String, i32) {
    let mut cmd = Command::new(bin());
    cmd.arg("agent");
    cmd.args(args);
    cmd.env("CLAUDEPOT_DATA_DIR", &env.data_path);
    let out = cmd.output().expect("failed to run claudepot");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn draft_creates_draft_record_and_no_scheduler_artifact() {
    let env = make_env();
    let (stdout, stderr, code) = run_agent(
        &env,
        &[
            "draft",
            "--json",
            "--name",
            "nightly-digest",
            "--cwd",
            "/tmp/proj",
            "--prompt",
            "summarize today's work",
        ],
    );
    assert_eq!(code, 0, "draft should succeed; stderr: {stderr}");

    // The store file exists and the record is a draft.
    let store_path = env.data_path.join("agents.json");
    assert!(store_path.exists(), "agents.json must be written");
    let raw = fs::read_to_string(&store_path).unwrap();
    let v: serde_json::Value = serde_json::from_str(&raw).unwrap();
    let agents = v["agents"].as_array().expect("agents array");
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0]["name"], "nightly-digest");
    assert_eq!(
        agents[0]["lifecycle"], "draft",
        "a drafted agent MUST be inert (lifecycle=draft)"
    );

    // The JSON stdout payload also reports the draft lifecycle.
    let payload: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(payload["lifecycle"], "draft");
    assert_eq!(payload["drafted_by"], "cli");

    // The load-bearing inertness check: `draft` must NOT create the
    // per-agent directory (which would hold a shim) — that only
    // happens when a human arms the agent in the GUI.
    let agents_dir = env.data_path.join("agents");
    assert!(
        !agents_dir.exists(),
        "draft must NOT materialize a per-agent scheduler/shim directory"
    );
}

#[test]
fn draft_accepts_claudepot_native_json() {
    let env = make_env();
    let spec = serde_json::json!({
        "name": "native-agent",
        "cwd": "/tmp/native",
        "prompt": "do the thing",
        "model": "claude-haiku-4-5",
        "allowed_tools": ["Read", "Grep"],
        "permission_mode": "bypassPermissions"
    });
    let spec_path = env.data_path.join("native-spec.json");
    fs::write(&spec_path, spec.to_string()).unwrap();

    let (stdout, stderr, code) = run_agent(
        &env,
        &[
            "draft",
            "--json",
            "--from-json",
            spec_path.to_str().unwrap(),
        ],
    );
    assert_eq!(
        code, 0,
        "native-JSON draft should succeed; stderr: {stderr}"
    );
    let payload: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(payload["name"], "native-agent");
    assert_eq!(payload["model"], "claude-haiku-4-5");
    assert_eq!(payload["permission_mode"], "bypassPermissions");
    assert_eq!(payload["lifecycle"], "draft");
}

#[test]
fn draft_accepts_agent_definition_shaped_json() {
    // PRD D2: `agent draft` accepts the SDK `AgentDefinition` shape
    // (description / prompt / tools / model). name + cwd come from
    // flags because the SDK shape carries neither.
    let env = make_env();
    let spec = serde_json::json!({
        "description": "Reviews diffs for security issues",
        "prompt": "You are a security reviewer.",
        "tools": ["Read", "Grep"],
        "model": "claude-haiku-4-5"
    });
    let spec_path = env.data_path.join("sdk-spec.json");
    fs::write(&spec_path, spec.to_string()).unwrap();

    let (stdout, stderr, code) = run_agent(
        &env,
        &[
            "draft",
            "--json",
            "--from-json",
            spec_path.to_str().unwrap(),
            "--name",
            "sec-review",
            "--cwd",
            "/tmp/repo",
        ],
    );
    assert_eq!(
        code, 0,
        "AgentDefinition-shaped draft should succeed; stderr: {stderr}"
    );
    let payload: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(payload["name"], "sec-review");
    assert_eq!(payload["cwd"], "/tmp/repo");
    assert_eq!(payload["prompt"], "You are a security reviewer.");
    assert_eq!(payload["model"], "claude-haiku-4-5");
    assert_eq!(payload["lifecycle"], "draft");
    let tools = payload["allowed_tools"].as_array().unwrap();
    assert_eq!(tools.len(), 2);
}

#[test]
fn list_and_show_round_trip_a_draft() {
    let env = make_env();
    let (_, stderr, code) = run_agent(
        &env,
        &[
            "draft",
            "--name",
            "round-trip",
            "--cwd",
            "/tmp/rt",
            "--prompt",
            "p",
        ],
    );
    assert_eq!(code, 0, "draft should succeed; stderr: {stderr}");

    let (list_out, _, list_code) = run_agent(&env, &["list", "--json"]);
    assert_eq!(list_code, 0);
    let list: serde_json::Value = serde_json::from_str(&list_out).unwrap();
    let arr = list.as_array().unwrap();
    assert_eq!(arr.len(), 1);
    assert_eq!(arr[0]["name"], "round-trip");
    assert_eq!(arr[0]["lifecycle"], "draft");

    let (show_out, _, show_code) = run_agent(&env, &["show", "--json", "round-trip"]);
    assert_eq!(show_code, 0);
    let shown: serde_json::Value = serde_json::from_str(&show_out).unwrap();
    assert_eq!(shown["name"], "round-trip");
    assert_eq!(shown["cwd"], "/tmp/rt");
}

#[test]
fn draft_rejects_bypass_without_allowed_tools() {
    // bypassPermissions with no whitelist is structurally unsafe;
    // the draft must be rejected so a human is never asked to arm
    // a broken record.
    let env = make_env();
    let (_, stderr, code) = run_agent(
        &env,
        &[
            "draft",
            "--name",
            "danger",
            "--cwd",
            "/tmp",
            "--prompt",
            "p",
            "--permission-mode",
            "bypassPermissions",
        ],
    );
    assert_ne!(code, 0, "bypass-without-tools draft must fail");
    assert!(
        stderr.contains("bypassPermissions"),
        "error should explain the bypassPermissions invariant; got: {stderr}"
    );
    // Nothing was persisted.
    assert!(!env.data_path.join("agents.json").exists());
}
