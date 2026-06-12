//! Lock-down test for the sessions.db split-brain guard: `codex index`
//! (and by shared helper, `mcp memory-server`) must honor
//! `CLAUDEPOT_DATA_DIR` when `--db` is not passed, resolving the SAME
//! sessions.db as the rest of the CLI and the GUI. Regression guard for
//! the audit finding where both verbs hardcoded `~/.claudepot/sessions.db`.

use std::process::Command;

use tempfile::TempDir;

fn bin() -> &'static str {
    env!("CARGO_BIN_EXE_claudepot")
}

#[test]
fn test_codex_index_honors_claudepot_data_dir_without_db_flag() {
    let data = TempDir::new().unwrap();
    let codex_home = TempDir::new().unwrap();
    std::fs::create_dir_all(codex_home.path().join("sessions")).unwrap();

    let out = Command::new(bin())
        .env("CLAUDEPOT_DATA_DIR", data.path())
        .env("CODEX_HOME", codex_home.path())
        .args(["codex", "index", "--json"])
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "codex index failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        data.path().join("sessions.db").exists(),
        "sessions.db was not created under CLAUDEPOT_DATA_DIR — the \
         default db path is not honoring the override"
    );
}
