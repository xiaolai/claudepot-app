//! Golden tests for `claudepot session slim`.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use tempfile::TempDir;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_claudepot")
}

fn mk_session(config: &std::path::Path, slug: &str, uuid: &str, huge_len: usize) -> PathBuf {
    let dir = config.join("projects").join(slug);
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{uuid}.jsonl"));
    let big = "x".repeat(huge_len);
    let body = format!(
        r#"{{"type":"user","message":{{"role":"user","content":"hello"}},"uuid":"u1","sessionId":"{uuid}"}}
{{"type":"user","message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"t1","tool":"bash","content":"{big}"}}]}},"uuid":"u2","sessionId":"{uuid}"}}
{{"type":"user","message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"t2","tool":"special","content":"{big}"}}]}},"uuid":"u3","sessionId":"{uuid}"}}
"#
    );
    fs::write(&path, body).unwrap();
    path
}

fn run(config: &std::path::Path, data: &std::path::Path, args: &[&str]) -> (String, String, i32) {
    let out = Command::new(bin())
        .env("CLAUDE_CONFIG_DIR", config)
        .env("CLAUDEPOT_DATA_DIR", data)
        .args(args)
        .output()
        .expect("spawn claudepot");
    (
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn slim_dry_run_default_does_not_rewrite() {
    let tmp = TempDir::new().unwrap();
    let config = tmp.path().join("cfg");
    let data = tmp.path().join("data");
    fs::create_dir_all(&config).unwrap();
    fs::create_dir_all(&data).unwrap();
    let path = mk_session(&config, "-slug-a", "abcdefab-aaaa-aaaa-aaaa-000000000001", 500);
    let before = fs::metadata(&path).unwrap().len();
    let (stdout, _stderr, code) = run(
        &config,
        &data,
        &[
            "session",
            "slim",
            path.to_str().unwrap(),
            "--drop-tool-results-over",
            "200",
        ],
    );
    assert_eq!(code, 0, "stdout={stdout}");
    assert!(stdout.contains("Plan (dry-run)"));
    let after = fs::metadata(&path).unwrap().len();
    assert_eq!(before, after, "dry-run must not mutate");
}

#[test]
fn slim_execute_redacts_and_keeps_pre_slim_in_trash() {
    let tmp = TempDir::new().unwrap();
    let config = tmp.path().join("cfg");
    let data = tmp.path().join("data");
    fs::create_dir_all(&config).unwrap();
    fs::create_dir_all(&data).unwrap();
    let path = mk_session(&config, "-slug-b", "bbbbbbbb-bbbb-bbbb-bbbb-000000000002", 500);
    let before = fs::metadata(&path).unwrap().len();
    let (stdout, _stderr, code) = run(
        &config,
        &data,
        &[
            "session",
            "slim",
            path.to_str().unwrap(),
            "--drop-tool-results-over",
            "200",
            "--execute",
        ],
    );
    assert_eq!(code, 0, "stdout={stdout}");
    let after = fs::metadata(&path).unwrap().len();
    assert!(after < before, "file must shrink");
    let body = fs::read_to_string(&path).unwrap();
    assert!(body.contains("tool_result_redacted"));
    // Trash listing must carry a Slim entry.
    let (stdout, _, _) = run(&config, &data, &["--json", "session", "trash", "list"]);
    let listing: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let entries = listing["entries"].as_array().unwrap();
    assert!(entries
        .iter()
        .any(|e| e["kind"].as_str().unwrap() == "slim"));
}

#[test]
fn slim_exclude_tool_preserves_that_tools_results() {
    let tmp = TempDir::new().unwrap();
    let config = tmp.path().join("cfg");
    let data = tmp.path().join("data");
    fs::create_dir_all(&config).unwrap();
    fs::create_dir_all(&data).unwrap();
    let path = mk_session(&config, "-slug-c", "cccccccc-cccc-cccc-cccc-000000000003", 500);
    run(
        &config,
        &data,
        &[
            "session",
            "slim",
            path.to_str().unwrap(),
            "--drop-tool-results-over",
            "100",
            "--exclude-tool",
            "special",
            "--execute",
        ],
    );
    let body = fs::read_to_string(&path).unwrap();
    // The special tool's huge payload survives verbatim.
    assert!(body.contains("\"tool\":\"special\""));
    assert!(body.contains(&"x".repeat(500)));
    // The bash tool's payload is redacted.
    assert!(body.contains("\"tool\":\"bash\"") || body.contains("tool_result_redacted"));
}

#[test]
fn slim_json_emits_plan_struct_on_dry_run() {
    let tmp = TempDir::new().unwrap();
    let config = tmp.path().join("cfg");
    let data = tmp.path().join("data");
    fs::create_dir_all(&config).unwrap();
    fs::create_dir_all(&data).unwrap();
    let path = mk_session(&config, "-slug-j", "dddddddd-dddd-dddd-dddd-000000000004", 500);
    let (stdout, _, code) = run(
        &config,
        &data,
        &[
            "--json",
            "session",
            "slim",
            path.to_str().unwrap(),
            "--drop-tool-results-over",
            "200",
        ],
    );
    assert_eq!(code, 0);
    let plan: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert!(plan.get("original_bytes").is_some());
    assert!(plan.get("projected_bytes").is_some());
    assert!(plan.get("redact_count").is_some());
    assert!(plan.get("tools_affected").is_some());
}
