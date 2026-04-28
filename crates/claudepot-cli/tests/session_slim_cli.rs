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
    let path = mk_session(
        &config,
        "-slug-a",
        "abcdefab-aaaa-aaaa-aaaa-000000000001",
        500,
    );
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
    let path = mk_session(
        &config,
        "-slug-b",
        "bbbbbbbb-bbbb-bbbb-bbbb-000000000002",
        500,
    );
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
    let path = mk_session(
        &config,
        "-slug-c",
        "cccccccc-cccc-cccc-cccc-000000000003",
        500,
    );
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
    let path = mk_session(
        &config,
        "-slug-j",
        "dddddddd-dddd-dddd-dddd-000000000004",
        500,
    );
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

// -- strip-images / strip-documents --------------------------------

fn mk_image_session(
    config: &std::path::Path,
    slug: &str,
    uuid: &str,
    img_b64_len: usize,
    doc_b64_len: usize,
) -> PathBuf {
    let dir = config.join("projects").join(slug);
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{uuid}.jsonl"));
    let img = "A".repeat(img_b64_len);
    let doc = "B".repeat(doc_b64_len);
    // Three lines: plain user text, top-level user image, top-level
    // user document. Mirrors the fixture layout.
    let body = format!(
        r#"{{"type":"user","uuid":"u1","sessionId":"{uuid}","message":{{"role":"user","content":"hi"}}}}
{{"type":"user","uuid":"u2","parentUuid":"u1","sessionId":"{uuid}","message":{{"role":"user","content":[{{"type":"image","source":{{"type":"base64","media_type":"image/png","data":"{img}"}}}}]}}}}
{{"type":"user","uuid":"u3","parentUuid":"u2","sessionId":"{uuid}","message":{{"role":"user","content":[{{"type":"document","source":{{"type":"base64","media_type":"application/pdf","data":"{doc}"}}}}]}}}}
"#
    );
    fs::write(&path, body).unwrap();
    path
}

#[test]
fn slim_strip_images_dry_run_prints_image_count() {
    let tmp = TempDir::new().unwrap();
    let config = tmp.path().join("cfg");
    let data = tmp.path().join("data");
    fs::create_dir_all(&config).unwrap();
    fs::create_dir_all(&data).unwrap();
    let path = mk_image_session(
        &config,
        "-slug-img",
        "eeeeeeee-eeee-eeee-eeee-000000000005",
        512,
        512,
    );
    let (stdout, _stderr, code) = run(
        &config,
        &data,
        &["session", "slim", path.to_str().unwrap(), "--strip-images"],
    );
    assert_eq!(code, 0, "stdout={stdout}");
    assert!(stdout.contains("Plan (dry-run)"));
    assert!(
        stdout.contains("Images redacted:     1"),
        "stdout missing image line:\n{stdout}"
    );
    assert!(
        !stdout.contains("Documents redacted:"),
        "docs line must not appear when --strip-documents is off"
    );
}

#[test]
fn slim_strip_documents_dry_run_prints_document_count() {
    let tmp = TempDir::new().unwrap();
    let config = tmp.path().join("cfg");
    let data = tmp.path().join("data");
    fs::create_dir_all(&config).unwrap();
    fs::create_dir_all(&data).unwrap();
    let path = mk_image_session(
        &config,
        "-slug-doc",
        "ffffffff-ffff-ffff-ffff-000000000006",
        512,
        512,
    );
    let (stdout, _stderr, code) = run(
        &config,
        &data,
        &[
            "session",
            "slim",
            path.to_str().unwrap(),
            "--strip-documents",
        ],
    );
    assert_eq!(code, 0, "stdout={stdout}");
    assert!(stdout.contains("Documents redacted:  1"));
    assert!(!stdout.contains("Images redacted:"));
}

#[test]
fn slim_strip_images_execute_rewrites_and_removes_base64() {
    let tmp = TempDir::new().unwrap();
    let config = tmp.path().join("cfg");
    let data = tmp.path().join("data");
    fs::create_dir_all(&config).unwrap();
    fs::create_dir_all(&data).unwrap();
    let path = mk_image_session(
        &config,
        "-slug-exec",
        "aaaaaaaa-bbbb-cccc-dddd-000000000007",
        4096,
        4096,
    );
    let before = fs::read_to_string(&path).unwrap();
    assert!(before.contains(&"A".repeat(4096)));
    let (stdout, _stderr, code) = run(
        &config,
        &data,
        &[
            "session",
            "slim",
            path.to_str().unwrap(),
            "--strip-images",
            "--strip-documents",
            "--execute",
        ],
    );
    assert_eq!(code, 0, "stdout={stdout}");
    assert!(stdout.contains("Images redacted:     1"));
    assert!(stdout.contains("Documents redacted:  1"));
    let after = fs::read_to_string(&path).unwrap();
    // Base64 payloads gone; stubs present.
    assert!(!after.contains(&"A".repeat(4096)));
    assert!(!after.contains(&"B".repeat(4096)));
    assert!(after.contains("\"[image]\""));
    assert!(after.contains("\"[document]\""));
    // UUID chain preserved.
    for uuid in ["u1", "u2", "u3"] {
        assert!(after.contains(&format!("\"{uuid}\"")));
    }
    // Trash carries the pre-slim snapshot.
    let (stdout, _, _) = run(&config, &data, &["--json", "session", "trash", "list"]);
    let listing: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let entries = listing["entries"].as_array().unwrap();
    assert!(entries
        .iter()
        .any(|e| e["kind"].as_str().unwrap() == "slim"));
}

#[test]
fn slim_all_without_filter_is_rejected() {
    let tmp = TempDir::new().unwrap();
    let config = tmp.path().join("cfg");
    let data = tmp.path().join("data");
    fs::create_dir_all(&config).unwrap();
    fs::create_dir_all(&data).unwrap();
    // --all with no filter: empty filter is rejected by the core planner.
    let (_stdout, stderr, code) = run(
        &config,
        &data,
        &["session", "slim", "--all", "--strip-images"],
    );
    assert_ne!(code, 0, "empty filter must fail; stderr={stderr}");
    assert!(
        stderr.contains("empty filter") || stderr.contains("criterion must be set"),
        "stderr should mention empty filter: {stderr}"
    );
}

#[test]
fn slim_all_dry_run_lists_top_entries() {
    let tmp = TempDir::new().unwrap();
    let config = tmp.path().join("cfg");
    let data = tmp.path().join("data");
    fs::create_dir_all(&config).unwrap();
    fs::create_dir_all(&data).unwrap();
    // Three image sessions with different ages. `--older-than 1s` matches all.
    mk_image_session(
        &config,
        "-slug-1",
        "01111111-1111-1111-1111-000000000001",
        2048,
        0,
    );
    mk_image_session(
        &config,
        "-slug-2",
        "02222222-2222-2222-2222-000000000002",
        2048,
        0,
    );
    mk_image_session(
        &config,
        "-slug-3",
        "03333333-3333-3333-3333-000000000003",
        2048,
        0,
    );
    // Wait a moment so `last_ts > 1s ago` matches.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let (stdout, stderr, code) = run(
        &config,
        &data,
        &[
            "session",
            "slim",
            "--all",
            "--older-than",
            "1s",
            "--strip-images",
        ],
    );
    assert_eq!(code, 0, "stderr={stderr}");
    assert!(stdout.contains("Plan (dry-run)"));
    assert!(stdout.contains("session(s)"));
    assert!(stdout.contains("Images to redact"));
    assert!(stdout.contains("Bytes saved"));
    assert!(stdout.contains("Top"));
    assert!(stdout.contains("Run with --execute"));
}

#[test]
fn slim_all_execute_slims_matching_sessions() {
    let tmp = TempDir::new().unwrap();
    let config = tmp.path().join("cfg");
    let data = tmp.path().join("data");
    fs::create_dir_all(&config).unwrap();
    fs::create_dir_all(&data).unwrap();
    let p1 = mk_image_session(
        &config,
        "-sA",
        "aaaaaaaa-aaaa-aaaa-aaaa-000000000001",
        2048,
        2048,
    );
    let p2 = mk_image_session(
        &config,
        "-sB",
        "bbbbbbbb-bbbb-bbbb-bbbb-000000000002",
        2048,
        2048,
    );
    let before1 = fs::read_to_string(&p1).unwrap();
    let before2 = fs::read_to_string(&p2).unwrap();
    assert!(before1.contains(&"A".repeat(2048)));
    assert!(before2.contains(&"A".repeat(2048)));
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let (stdout, stderr, code) = run(
        &config,
        &data,
        &[
            "session",
            "slim",
            "--all",
            "--older-than",
            "1s",
            "--strip-images",
            "--execute",
        ],
    );
    assert_eq!(code, 0, "stderr={stderr}");
    assert!(stdout.contains("Bulk slim: 2 succeeded"));
    let after1 = fs::read_to_string(&p1).unwrap();
    let after2 = fs::read_to_string(&p2).unwrap();
    assert!(!after1.contains(&"A".repeat(2048)));
    assert!(!after2.contains(&"A".repeat(2048)));
    // Two separate trash entries.
    let (stdout, _, _) = run(&config, &data, &["--json", "session", "trash", "list"]);
    let listing: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let slim_entries: Vec<_> = listing["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["kind"].as_str() == Some("slim"))
        .collect();
    assert_eq!(slim_entries.len(), 2);
}

#[test]
fn slim_single_target_rejects_bulk_only_filter_flags() {
    // `--older-than` on a single-target slim is nonsense — it would
    // parse but silently be ignored. Handler enforces the "requires
    // --all" rule explicitly (clap `requires` doesn't fire for a
    // default-false bool flag the way you'd expect).
    let tmp = TempDir::new().unwrap();
    let config = tmp.path().join("cfg");
    let data = tmp.path().join("data");
    fs::create_dir_all(&config).unwrap();
    fs::create_dir_all(&data).unwrap();
    let (_stdout, stderr, code) = run(
        &config,
        &data,
        &["session", "slim", "some-target.jsonl", "--older-than", "7d"],
    );
    assert_ne!(code, 0);
    assert!(
        stderr.contains("requires --all") || stderr.contains("bulk-only"),
        "stderr should explain that --older-than requires --all: {stderr}"
    );
}

#[test]
fn slim_all_conflicts_with_positional_target() {
    let tmp = TempDir::new().unwrap();
    let config = tmp.path().join("cfg");
    let data = tmp.path().join("data");
    fs::create_dir_all(&config).unwrap();
    fs::create_dir_all(&data).unwrap();
    let (_stdout, stderr, code) = run(
        &config,
        &data,
        &[
            "session",
            "slim",
            "some-target.jsonl",
            "--all",
            "--older-than",
            "7d",
        ],
    );
    assert_ne!(code, 0);
    assert!(
        stderr.contains("cannot be used with") || stderr.contains("conflicts"),
        "stderr should name the conflict: {stderr}"
    );
}

#[test]
fn slim_baseline_without_new_flags_unchanged() {
    // When neither --strip-images nor --strip-documents is passed and
    // no oversized tool_results exist, the output shape is identical
    // to the pre-flag behavior.
    let tmp = TempDir::new().unwrap();
    let config = tmp.path().join("cfg");
    let data = tmp.path().join("data");
    fs::create_dir_all(&config).unwrap();
    fs::create_dir_all(&data).unwrap();
    let path = mk_image_session(
        &config,
        "-slug-base",
        "11111111-2222-3333-4444-000000000008",
        256,
        256,
    );
    let (stdout, _stderr, code) = run(&config, &data, &["session", "slim", path.to_str().unwrap()]);
    assert_eq!(code, 0);
    assert!(stdout.contains("Plan (dry-run)"));
    assert!(!stdout.contains("Images redacted:"));
    assert!(!stdout.contains("Documents redacted:"));
}
