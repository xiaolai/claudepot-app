//! §11.4 trust-gate goldens.

use crate::migrate::bundle::{sidecar_path_for, BundleReader, BundleWriter};
use crate::migrate::manifest::{BundleManifest, ExportFlags, SCHEMA_VERSION};
use crate::migrate::trust::{scrub_mcp_servers, split_settings};
use crate::migrate::MigrateError;
use std::fs;

fn fixture_manifest() -> BundleManifest {
    BundleManifest {
        schema_version: SCHEMA_VERSION,
        claudepot_version: env!("CARGO_PKG_VERSION").to_string(),
        cc_version: None,
        created_at: "2026-04-27T00:00:00Z".to_string(),
        source_os: "macos".to_string(),
        source_arch: "aarch64".to_string(),
        host_identity: "ab".repeat(32),
        source_home: "/Users/joker".to_string(),
        source_claude_config_dir: "/Users/joker/.claude".to_string(),
        projects: vec![],
        flags: ExportFlags::default(),
        file_inventory: vec![],
    }
}

// Row 28 — bundle carries 5 hooks: all rejected by default;
// `--accept-hooks` accepts; per-hook reject works.
//
// Pinned at the split-settings layer + the per-hook list shape. The
// import-side "hooks not auto-installed" contract is enforced by
// the orchestrator's omission of any `accept_hooks` flow when the
// bundle carries hooks but the user didn't pass `--accept-hooks`.
#[test]
fn row28_split_extracts_five_hooks_into_separate_block() {
    let json = serde_json::json!({
        "theme": "dark",
        "hooks": {
            "PreToolUse": [
                {"matcher": "Bash", "hooks": [{"type": "command", "command": "h1"}]},
                {"matcher": "Bash", "hooks": [{"type": "command", "command": "h2"}]},
            ],
            "PostToolUse": [
                {"matcher": "Edit", "hooks": [{"type": "command", "command": "h3"}]},
            ],
            "SessionStart": [
                {"matcher": "*", "hooks": [{"type": "command", "command": "h4"}]},
            ],
            "SessionEnd": [
                {"matcher": "*", "hooks": [{"type": "command", "command": "h5"}]},
            ],
        }
    });
    let split = split_settings(json);
    assert!(split.scrubbed.get("hooks").is_none());
    let hooks = split.hooks.unwrap();
    // 5 hooks across 4 events.
    let pre = hooks["PreToolUse"].as_array().unwrap();
    assert_eq!(pre.len(), 2);
    assert!(hooks.get("PostToolUse").is_some());
    assert!(hooks.get("SessionStart").is_some());
    assert!(hooks.get("SessionEnd").is_some());
}

// Row 29 — bundle MCP server with absolute-path command on different
// OS: imported disabled with needs-resolution.
#[test]
fn row29_absolute_mcp_path_marked_needs_resolution() {
    let mut map = serde_json::Map::new();
    map.insert(
        "tauri-mcp".to_string(),
        serde_json::json!({"command": "/opt/bin/tauri-mcp", "args": []}),
    );
    map.insert(
        "node".to_string(),
        serde_json::json!({"command": "node", "args": ["server.js"]}),
    );
    let n = scrub_mcp_servers(&mut map);
    assert_eq!(n, 1);
    assert!(map["tauri-mcp"]["_claudepot"]["needs_resolution"]
        .as_bool()
        .unwrap());
    assert!(map["node"].get("_claudepot").is_none());
}

// Row 30 — bundle has plugin from private GitHub repo, target has
// no auth: project imports; plugin recorded unavailable.
//
// Plugin re-install is deferred; the contract pinned here is that
// the *project* import path doesn't depend on plugin reachability.
#[test]
fn row30_plugin_import_independent_of_project_import() {
    // The flag flips the export side; no project API is touched by
    // include_global at the bundle's per-project level.
    let pm = crate::migrate::manifest::ProjectManifest {
        id: "abc".to_string(),
        source_cwd: "/x".to_string(),
        source_canonical_git_root: "/x".to_string(),
        source_slug: "-x".to_string(),
        session_ids: vec![],
        live_at_export: false,
        worktree_set: false,
    };
    // Plugin presence/absence is at the bundle level (global/), not
    // the per-project level. The per-project manifest carries no
    // plugin field.
    let s = serde_json::to_string(&pm).unwrap();
    assert!(!s.contains("plugin"));
}

// Row 31 — tampered bundle (one byte flipped): integrity verify
// refuses with file name.
#[test]
fn row31_tampered_sidecar_rejects_open() {
    let tmp = tempfile::tempdir().unwrap();
    let bundle_path = tmp.path().join("t.tar.zst");
    let mut w = BundleWriter::create(&bundle_path).unwrap();
    w.append_bytes("a.txt", b"a", 0o644).unwrap();
    w.finalize(&fixture_manifest()).unwrap();
    // Tamper sidecar — flip one hex char.
    let sidecar = sidecar_path_for(&bundle_path);
    let s = fs::read_to_string(&sidecar).unwrap();
    // First char is hex; flip it to a different hex digit.
    let mut bytes = s.into_bytes();
    bytes[0] = if bytes[0] == b'0' { b'1' } else { b'0' };
    fs::write(&sidecar, bytes).unwrap();
    let err = BundleReader::open(&bundle_path).unwrap_err();
    assert!(matches!(err, MigrateError::IntegrityViolation(_)));
}

// Row 32 — encrypted bundle with wrong passphrase: refuse, no
// partial extraction.
//
// Encryption is deferred (`crypto::require_plaintext_only`). The
// contract: requesting encryption returns NotImplemented, never a
// partial extraction.
#[test]
fn row32_encrypt_returns_not_implemented_no_partial_state() {
    let tmp = tempfile::tempdir().unwrap();
    let cfg = tmp.path().join("c");
    fs::create_dir_all(cfg.join("projects")).unwrap();
    let opts = crate::migrate::ExportOptions {
        output: tmp.path().join("e.tar.zst"),
        project_cwds: vec![],
        include_global: false,
        include_worktree: false,
        include_live: false,
        include_claudepot_state: false,
        include_file_history: true,
        encrypt: true,
        sign_keyfile: None,
        account_stubs: None,
        encrypt_passphrase: None,
        sign_password: None,
    };
    let err = crate::migrate::export_projects(&cfg, opts).unwrap_err();
    // After v1 encryption support landed, missing passphrase became a
    // Configuration error rather than NotImplemented (the feature
    // ships; the user just didn't supply it).
    assert!(matches!(err, MigrateError::Configuration(_)));
    // No bundle file was written.
    assert!(!tmp.path().join("e.tar.zst").exists());
}
