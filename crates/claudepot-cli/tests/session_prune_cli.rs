//! Golden tests for `claudepot session prune`.
//!
//! Dry-run is the default; `--execute` is required to actually move
//! files. `--json` round-trips the PrunePlan struct unchanged. Both
//! `CLAUDE_CONFIG_DIR` and `CLAUDEPOT_DATA_DIR` are isolated per test.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use tempfile::TempDir;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_claudepot")
}

struct TestEnv {
    _config: TempDir,
    _data: TempDir,
    config_path: PathBuf,
    data_path: PathBuf,
}

fn make_env() -> TestEnv {
    let config = TempDir::new().unwrap();
    let data = TempDir::new().unwrap();
    let config_path = config.path().to_path_buf();
    let data_path = data.path().to_path_buf();
    fs::create_dir_all(config_path.join("projects")).unwrap();
    TestEnv {
        _config: config,
        _data: data,
        config_path,
        data_path,
    }
}

fn write_session(env: &TestEnv, slug: &str, uuid: &str, body: &str) -> PathBuf {
    let dir = env.config_path.join("projects").join(slug);
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{uuid}.jsonl"));
    fs::write(&path, body).unwrap();
    path
}

/// A JSONL body with a `summary` row + one `user` turn carrying a
/// UTC timestamp. Pass `ts` to control `last_ts`.
fn mk_body(uuid: &str, cwd: &str, ts: &str, size_padding_bytes: usize) -> String {
    let padding = "x".repeat(size_padding_bytes);
    format!(
        r#"{{"type":"summary","summary":"t","leafUuid":"{uuid}"}}
{{"type":"user","message":{{"role":"user","content":"hello {padding}"}},"uuid":"u1","sessionId":"{uuid}","cwd":"{cwd}","timestamp":"{ts}"}}
"#
    )
}

fn run(env: &TestEnv, args: &[&str]) -> (String, String, i32) {
    let out = Command::new(bin())
        .env("CLAUDE_CONFIG_DIR", &env.config_path)
        .env("CLAUDEPOT_DATA_DIR", &env.data_path)
        .args(args)
        .output()
        .expect("spawn claudepot");
    (
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
        out.status.code().unwrap_or(-1),
    )
}

fn count_files(dir: &Path) -> usize {
    if !dir.exists() {
        return 0;
    }
    let mut n = 0;
    for entry in fs::read_dir(dir).unwrap() {
        let e = entry.unwrap();
        if e.file_type().unwrap().is_dir() {
            n += count_files(&e.path());
        } else {
            n += 1;
        }
    }
    n
}

#[test]
fn prune_dry_run_default_does_not_move_files() {
    let env = make_env();
    let session_path = write_session(
        &env,
        "-repo-a",
        "11111111-1111-1111-1111-111111111111",
        // Old enough (2020) to satisfy any reasonable older-than
        &mk_body(
            "11111111-1111-1111-1111-111111111111",
            "/tmp/repo-a",
            "2020-01-01T00:00:00Z",
            0,
        ),
    );
    let (stdout, _stderr, code) = run(&env, &["session", "prune", "--older-than", "30d"]);
    assert_eq!(code, 0, "stdout={stdout}");
    assert!(stdout.contains("Plan (dry-run)"));
    assert!(stdout.contains("--execute"));
    // File still exists.
    assert!(session_path.exists());
    // Trash dir doesn't exist.
    assert!(!env.data_path.join("trash").exists());
}

#[test]
fn prune_execute_moves_matching_files_to_trash() {
    let env = make_env();
    let session_path = write_session(
        &env,
        "-repo-b",
        "22222222-2222-2222-2222-222222222222",
        &mk_body(
            "22222222-2222-2222-2222-222222222222",
            "/tmp/repo-b",
            "2020-01-01T00:00:00Z",
            0,
        ),
    );
    let (stdout, _stderr, code) = run(
        &env,
        &["session", "prune", "--older-than", "30d", "--execute"],
    );
    assert_eq!(code, 0, "stdout={stdout}");
    assert!(!session_path.exists(), "file should be gone");
    let trash_root = env.data_path.join("trash/sessions");
    assert!(trash_root.exists());
    assert!(count_files(&trash_root) >= 2, "file + manifest in trash");
}

#[test]
fn prune_json_emits_plan_shape() {
    let env = make_env();
    write_session(
        &env,
        "-repo-j",
        "33333333-3333-3333-3333-333333333333",
        &mk_body(
            "33333333-3333-3333-3333-333333333333",
            "/tmp/repo-j",
            "2020-01-01T00:00:00Z",
            0,
        ),
    );
    let (stdout, _stderr, code) = run(&env, &["--json", "session", "prune", "--older-than", "30d"]);
    assert_eq!(code, 0);
    let plan: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert!(plan.get("entries").is_some());
    assert!(plan.get("total_bytes").is_some());
    let entries = plan["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert!(entries[0].get("size_bytes").is_some());
    assert!(entries[0].get("file_path").is_some());
}

#[test]
fn prune_without_any_filter_rejects_with_clear_error() {
    let env = make_env();
    // With no filter, plan_prune's validate() returns EmptyFilter.
    let (_stdout, stderr, code) = run(&env, &["session", "prune"]);
    assert_ne!(code, 0);
    assert!(
        stderr.contains("empty filter") || stderr.contains("criterion"),
        "stderr={stderr}"
    );
}

#[test]
fn trash_list_empty_prints_friendly_message() {
    let env = make_env();
    let (stdout, _stderr, code) = run(&env, &["session", "trash", "list"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("Trash is empty"), "stdout={stdout}");
}

#[test]
fn trash_list_after_prune_shows_the_moved_entry() {
    let env = make_env();
    write_session(
        &env,
        "-repo-l",
        "44444444-4444-4444-4444-444444444444",
        &mk_body(
            "44444444-4444-4444-4444-444444444444",
            "/tmp/repo-l",
            "2020-01-01T00:00:00Z",
            0,
        ),
    );
    run(
        &env,
        &["session", "prune", "--older-than", "30d", "--execute"],
    );
    let (stdout, _stderr, code) = run(&env, &["--json", "session", "trash", "list"]);
    assert_eq!(code, 0);
    let listing: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let entries = listing["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["kind"].as_str().unwrap(), "prune");
}

#[test]
fn trash_restore_puts_the_file_back() {
    let env = make_env();
    let session_path = write_session(
        &env,
        "-repo-r",
        "55555555-5555-5555-5555-555555555555",
        &mk_body(
            "55555555-5555-5555-5555-555555555555",
            "/tmp/repo-r",
            "2020-01-01T00:00:00Z",
            0,
        ),
    );
    run(
        &env,
        &["session", "prune", "--older-than", "30d", "--execute"],
    );
    assert!(!session_path.exists());
    // List to get the batch id.
    let (stdout, _stderr, _) = run(&env, &["--json", "session", "trash", "list"]);
    let listing: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    let id = listing["entries"][0]["id"].as_str().unwrap().to_string();
    let (stdout, _stderr, code) = run(&env, &["session", "trash", "restore", &id]);
    assert_eq!(code, 0, "stdout={stdout}");
    assert!(session_path.exists());
}

#[test]
fn trash_empty_with_yes_flag_clears_everything() {
    let env = make_env();
    write_session(
        &env,
        "-repo-e",
        "66666666-6666-6666-6666-666666666666",
        &mk_body(
            "66666666-6666-6666-6666-666666666666",
            "/tmp/repo-e",
            "2020-01-01T00:00:00Z",
            0,
        ),
    );
    run(
        &env,
        &["session", "prune", "--older-than", "30d", "--execute"],
    );
    let (stdout, _stderr, code) = run(&env, &["-y", "session", "trash", "empty"]);
    assert_eq!(code, 0, "stdout={stdout}");
    let (stdout, _stderr, _) = run(&env, &["--json", "session", "trash", "list"]);
    let listing: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert!(listing["entries"].as_array().unwrap().is_empty());
}
