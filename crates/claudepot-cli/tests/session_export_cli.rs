//! Golden tests for the extended `session export` verb.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use tempfile::TempDir;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_claudepot")
}

fn write_session(config: &std::path::Path, slug: &str, uuid: &str) -> PathBuf {
    let dir = config.join("projects").join(slug);
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{uuid}.jsonl"));
    let body = format!(
        r#"{{"type":"user","message":{{"role":"user","content":"hello sk-ant-oat01-LEAK1234 world"}},"uuid":"u1","sessionId":"{uuid}","cwd":"/tmp/t","timestamp":"2026-04-10T10:00:00Z"}}
{{"type":"assistant","message":{{"role":"assistant","content":[{{"type":"text","text":"hi"}}]}},"uuid":"a1","sessionId":"{uuid}","cwd":"/tmp/t","timestamp":"2026-04-10T10:00:01Z"}}
"#
    );
    fs::write(&path, body).unwrap();
    path
}

fn run(config: &std::path::Path, data: &std::path::Path, args: &[&str]) -> (String, String, i32) {
    let out = Command::new(bin())
        .env("CLAUDE_CONFIG_DIR", config)
        .env("CLAUDEPOT_DATA_DIR", data)
        .env_remove("GITHUB_TOKEN")
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
fn export_markdown_to_file_backward_compatible() {
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("cfg");
    let data = tmp.path().join("data");
    fs::create_dir_all(&cfg).unwrap();
    fs::create_dir_all(&data).unwrap();
    let session = write_session(&cfg, "-t", "11111111-1111-1111-1111-111111111111");
    let out_file = tmp.path().join("out.md");
    let (_stdout, _stderr, code) = run(
        &cfg,
        &data,
        &[
            "session",
            "export",
            session.to_str().unwrap(),
            "--format",
            "md",
            "--output",
            out_file.to_str().unwrap(),
        ],
    );
    assert_eq!(code, 0);
    let body = fs::read_to_string(&out_file).unwrap();
    assert!(body.contains("# Session"));
    assert!(!body.contains("sk-ant-oat01-LEAK1234"));
    assert!(body.contains("sk-ant-***"));
}

#[test]
fn export_html_to_file_writes_self_contained_html() {
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("cfg");
    let data = tmp.path().join("data");
    fs::create_dir_all(&cfg).unwrap();
    fs::create_dir_all(&data).unwrap();
    let session = write_session(&cfg, "-h", "22222222-2222-2222-2222-222222222222");
    let out_file = tmp.path().join("out.html");
    let (_stdout, _stderr, code) = run(
        &cfg,
        &data,
        &[
            "session",
            "export",
            session.to_str().unwrap(),
            "--format",
            "html",
            "--to",
            "file",
            "--output",
            out_file.to_str().unwrap(),
            "--html-no-js",
        ],
    );
    assert_eq!(code, 0);
    let body = fs::read_to_string(&out_file).unwrap();
    assert!(body.starts_with("<!doctype html>"));
    assert!(!body.contains("<script>"));
    assert!(body.contains("prefers-color-scheme"));
    assert!(!body.contains("sk-ant-oat01-LEAK1234"));
}

#[test]
fn export_to_gist_without_token_fails_with_clear_error() {
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("cfg");
    let data = tmp.path().join("data");
    fs::create_dir_all(&cfg).unwrap();
    fs::create_dir_all(&data).unwrap();
    let session = write_session(&cfg, "-g", "33333333-3333-3333-3333-333333333333");
    let (_stdout, stderr, code) = run(
        &cfg,
        &data,
        &[
            "session",
            "export",
            session.to_str().unwrap(),
            "--format",
            "md",
            "--to",
            "gist",
        ],
    );
    assert_ne!(code, 0);
    assert!(
        stderr.contains("GITHUB_TOKEN") || stderr.contains("GitHub"),
        "stderr={stderr}"
    );
}

#[test]
fn export_to_file_without_output_fails() {
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("cfg");
    let data = tmp.path().join("data");
    fs::create_dir_all(&cfg).unwrap();
    fs::create_dir_all(&data).unwrap();
    let session = write_session(&cfg, "-f", "44444444-4444-4444-4444-444444444444");
    let (_stdout, stderr, code) = run(
        &cfg,
        &data,
        &[
            "session",
            "export",
            session.to_str().unwrap(),
            "--format",
            "md",
            "--to",
            "file",
        ],
    );
    assert_ne!(code, 0);
    assert!(stderr.contains("--output"), "stderr={stderr}");
}

#[test]
fn export_redact_paths_hash_rewrites_abs_paths() {
    let tmp = TempDir::new().unwrap();
    let cfg = tmp.path().join("cfg");
    let data = tmp.path().join("data");
    fs::create_dir_all(&cfg).unwrap();
    fs::create_dir_all(&data).unwrap();
    let session = write_session(&cfg, "-p", "55555555-5555-5555-5555-555555555555");
    let out_file = tmp.path().join("out.md");
    run(
        &cfg,
        &data,
        &[
            "session",
            "export",
            session.to_str().unwrap(),
            "--format",
            "md",
            "--output",
            out_file.to_str().unwrap(),
            "--redact-paths",
            "hash",
        ],
    );
    let body = fs::read_to_string(&out_file).unwrap();
    // /tmp/t is the fixture's cwd; with hash rewrite, it either
    // appears in <path:…> form or doesn't appear at all. Either way
    // the raw path should not survive verbatim.
    let has_marker = body.contains("<path:");
    let raw_survives = body.contains("/tmp/t ") || body.contains("/tmp/t\n");
    assert!(
        has_marker || !raw_survives,
        "path redaction left /tmp/t verbatim: {body}"
    );
}
