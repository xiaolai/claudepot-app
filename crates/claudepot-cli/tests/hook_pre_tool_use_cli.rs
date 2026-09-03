//! End-to-end for `claudepot hook pre-tool-use`, the verb Claude Code
//! runs before every tool call while a permission grant is live.
//!
//! Exercised through the real binary with a real stdin payload, because
//! the whole contract is about bytes on stdout: exactly the allow
//! decision when a live grant covers the session's `cwd`, and exactly
//! nothing otherwise — including when the grants file is corrupt, which
//! must also leave that file untouched.

use std::process::{Command, Stdio};

use tempfile::TempDir;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_claudepot")
}

fn run(data_dir: &std::path::Path, payload: &str) -> (String, String, bool) {
    use std::io::Write;
    let mut child = Command::new(bin())
        .env("CLAUDEPOT_DATA_DIR", data_dir)
        .args(["hook", "pre-tool-use"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.success(),
    )
}

fn grants_file(project: &str, expires_at: Option<&str>) -> String {
    let expires = expires_at.map_or("null".to_string(), |e| format!("\"{e}\""));
    format!(
        r#"{{"schema_version":2,"grants":[{{"project_path":"{project}",
            "granted_at":"2026-01-01T00:00:00Z","expires_at":{expires}}}]}}"#
    )
}

fn payload(cwd: &str) -> String {
    format!(
        r#"{{"session_id":"s1","transcript_path":"/x.jsonl","cwd":"{cwd}",
            "hook_event_name":"PreToolUse","tool_name":"Bash",
            "tool_input":{{"command":"python3 -c 'print(6*7)'"}},"a_field_from_next_month":1}}"#
    )
}

#[test]
fn a_live_grant_covering_the_cwd_prints_the_allow_decision() {
    let data = TempDir::new().unwrap();
    let project = TempDir::new().unwrap();
    let root = project.path().to_string_lossy().into_owned();
    std::fs::write(
        data.path().join("permission-grants.json"),
        grants_file(&root, None),
    )
    .unwrap();

    let sub = project.path().join("src");
    std::fs::create_dir_all(&sub).unwrap();
    let (stdout, stderr, ok) = run(data.path(), &payload(&sub.to_string_lossy()));
    assert!(ok, "stderr: {stderr}");
    assert!(
        stderr.is_empty(),
        "the hook must be silent on stderr: {stderr}"
    );
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(v["hookSpecificOutput"]["hookEventName"], "PreToolUse");
    assert_eq!(v["hookSpecificOutput"]["permissionDecision"], "allow");
}

#[test]
fn a_session_outside_every_granted_project_gets_silence() {
    let data = TempDir::new().unwrap();
    let project = TempDir::new().unwrap();
    let elsewhere = TempDir::new().unwrap();
    std::fs::write(
        data.path().join("permission-grants.json"),
        grants_file(&project.path().to_string_lossy(), None),
    )
    .unwrap();
    let (stdout, stderr, ok) = run(data.path(), &payload(&elsewhere.path().to_string_lossy()));
    assert!(ok);
    assert!(stdout.is_empty(), "got: {stdout}");
    assert!(stderr.is_empty(), "got: {stderr}");
}

#[test]
fn an_expired_grant_gets_silence() {
    let data = TempDir::new().unwrap();
    let project = TempDir::new().unwrap();
    std::fs::write(
        data.path().join("permission-grants.json"),
        grants_file(
            &project.path().to_string_lossy(),
            Some("2026-01-01T00:00:01Z"),
        ),
    )
    .unwrap();
    let (stdout, _, ok) = run(data.path(), &payload(&project.path().to_string_lossy()));
    assert!(ok);
    assert!(stdout.is_empty(), "got: {stdout}");
}

#[test]
fn a_corrupt_grants_file_gets_silence_and_is_left_exactly_as_it_was() {
    // The hook runs inside every tool call as the user. It must not
    // recover, rename, or log — the GUI's tick owns that.
    let data = TempDir::new().unwrap();
    let path = data.path().join("permission-grants.json");
    std::fs::write(&path, b"{not json").unwrap();
    let (stdout, stderr, ok) = run(data.path(), &payload("/tmp"));
    assert!(ok);
    assert!(stdout.is_empty(), "got: {stdout}");
    assert!(stderr.is_empty(), "got: {stderr}");
    assert_eq!(std::fs::read(&path).unwrap(), b"{not json");
    let leftovers: Vec<_> = std::fs::read_dir(data.path())
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(leftovers, vec!["permission-grants.json".to_string()]);
}

#[test]
fn no_grants_file_at_all_gets_silence() {
    let data = TempDir::new().unwrap();
    let (stdout, stderr, ok) = run(data.path(), &payload("/tmp"));
    assert!(ok);
    assert!(stdout.is_empty());
    assert!(stderr.is_empty());
}

#[test]
fn an_unparseable_payload_gets_silence() {
    let data = TempDir::new().unwrap();
    std::fs::write(
        data.path().join("permission-grants.json"),
        grants_file("/tmp", None),
    )
    .unwrap();
    let (stdout, stderr, ok) = run(data.path(), "this is not json");
    assert!(ok);
    assert!(stdout.is_empty());
    assert!(stderr.is_empty());
}
