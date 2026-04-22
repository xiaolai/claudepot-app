//! Golden tests for `claudepot session search`. Verifies:
//! - `--json` output includes the `score` field on every hit.
//! - Ranked output: a phrase match outranks a substring match even if
//!   the substring match is newer.
//! - Human-format output includes a `score=` column.
//! - `--limit` is respected.
//!
//! The CLI reads sessions from `$CLAUDE_CONFIG_DIR/projects/<slug>/<uuid>.jsonl`
//! and caches metadata under `$CLAUDEPOT_DATA_DIR/sessions.db`. Both
//! are pointed at fresh tempdirs per test to keep the suite hermetic.

use std::fs;
use std::path::Path;
use std::process::Command;

use tempfile::TempDir;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_claudepot")
}

fn write_jsonl(path: &Path, lines: &[&str]) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    let mut body = String::new();
    for l in lines {
        body.push_str(l);
        body.push('\n');
    }
    fs::write(path, body).unwrap();
}

struct TestEnv {
    _config: TempDir,
    _data: TempDir,
    config_path: std::path::PathBuf,
    data_path: std::path::PathBuf,
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

fn run_search(env: &TestEnv, extra: &[&str]) -> (String, String, i32) {
    let out = Command::new(bin())
        .env("CLAUDE_CONFIG_DIR", &env.config_path)
        .env("CLAUDEPOT_DATA_DIR", &env.data_path)
        // Isolate from real keychain / real CC state.
        .env_remove("CLAUDEPOT_KEYCHAIN_SERVICE")
        .args(extra)
        .output()
        .expect("spawn claudepot");
    (
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
        out.status.code().unwrap_or(-1),
    )
}

/// Writes one session per slug. Each line is a full user turn whose
/// `content` is the given text.
fn write_session(env: &TestEnv, slug: &str, uuid: &str, user_text: &str) {
    let path = env
        .config_path
        .join("projects")
        .join(slug)
        .join(format!("{uuid}.jsonl"));
    let line = format!(
        r#"{{"type":"summary","summary":"t","leafUuid":"{uuid}"}}
{{"type":"user","message":{{"role":"user","content":{content}}},"uuid":"u1","sessionId":"{uuid}","cwd":"/tmp/{slug}","timestamp":"2026-04-10T10:00:00Z"}}"#,
        uuid = uuid,
        slug = slug,
        content = serde_json::Value::String(user_text.to_string()),
    );
    write_jsonl(&path, &line.lines().collect::<Vec<_>>());
}

#[test]
fn search_json_includes_score_field_on_every_hit() {
    let env = make_env();
    write_session(
        &env,
        "-tmp-one",
        "11111111-1111-1111-1111-111111111111",
        "please investigate auth bug",
    );
    write_session(
        &env,
        "-tmp-two",
        "22222222-2222-2222-2222-222222222222",
        "unrelated content about databases",
    );
    let (stdout, _stderr, code) = run_search(&env, &["--json", "session", "search", "auth"]);
    assert_eq!(code, 0, "exit 0, got stdout={stdout}");
    let hits: serde_json::Value = serde_json::from_str(&stdout).expect("valid json");
    let arr = hits.as_array().expect("array");
    assert_eq!(arr.len(), 1);
    let h = &arr[0];
    assert!(
        h.get("score").is_some(),
        "hit missing score field: {h}"
    );
    let score = h["score"].as_f64().expect("score is number");
    assert!(score > 0.0 && score <= 1.0, "score in bounds: {score}");
}

#[test]
fn search_ranks_phrase_above_substring_match() {
    let env = make_env();
    // substring hit: "auth" is inside "unauthorized"
    write_session(
        &env,
        "-sub",
        "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
        "unauthorized access detected",
    );
    // phrase hit: "auth" as a standalone word.  Both sessions carry the
    // same `timestamp` in the synthetic JSONL, so `last_ts` ties, and
    // score alone decides the order.
    write_session(
        &env,
        "-phrase",
        "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb",
        "please review auth settings",
    );

    let (stdout, _stderr, code) = run_search(&env, &["--json", "session", "search", "auth"]);
    assert_eq!(code, 0);
    let hits: serde_json::Value = serde_json::from_str(&stdout).expect("valid json");
    let arr = hits.as_array().expect("array");
    assert_eq!(arr.len(), 2);
    let first_score = arr[0]["score"].as_f64().unwrap();
    let second_score = arr[1]["score"].as_f64().unwrap();
    assert!(
        first_score > second_score,
        "expected ranked desc, got {first_score} then {second_score}"
    );
    // The phrase session (review auth) comes first.
    let first_id = arr[0]["session_id"].as_str().unwrap();
    assert_eq!(first_id, "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb");
}

#[test]
fn search_human_output_shows_score_column() {
    let env = make_env();
    write_session(
        &env,
        "-h",
        "cccccccc-cccc-cccc-cccc-cccccccccccc",
        "please investigate auth bug",
    );
    let (stdout, _stderr, code) = run_search(&env, &["session", "search", "auth"]);
    assert_eq!(code, 0, "stdout={stdout}");
    assert!(stdout.contains("score="), "expected score= in: {stdout}");
    assert!(stdout.contains("match:"), "expected match: line: {stdout}");
}

#[test]
fn search_limit_respected() {
    let env = make_env();
    for i in 0..5u32 {
        let slug = format!("-slug-{i}");
        let uuid = format!("{i:08x}-0000-0000-0000-000000000000");
        write_session(&env, &slug, &uuid, &format!("widget number {i}"));
    }
    let (stdout, _stderr, code) =
        run_search(&env, &["--json", "session", "search", "widget", "--limit", "2"]);
    assert_eq!(code, 0);
    let hits: serde_json::Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(hits.as_array().unwrap().len(), 2);
}
