//! `--include-claudepot-state` — Claudepot's own state stubs.
//!
//! See `dev-docs/project-migrate-spec.md` §3.2 (`claudepot/`) and
//! §16 Q2 — account stubs are LOG-ONLY on import, never auto-insert.
//!
//! What travels:
//!   - Account stubs: `(uuid, email, org, verification shape)` —
//!     **NEVER** `has_cli_credentials`-true blob references, raw
//!     tokens, or anything that would let the target unlock the
//!     keychain.
//!   - Protected paths: the user's curated project filter list.
//!   - Preferences: theme, density, etc.
//!   - Artifact lifecycle policy.
//!
//! What does **not** travel:
//!   - Keychain entries (per-account credentials).
//!   - `has_cli_credentials` / `has_desktop_profile` flags
//!     (recomputed on the target after re-login).
//!   - Verification timestamps (target re-verifies).

use crate::migrate::bundle::BundleWriter;
use crate::migrate::error::MigrateError;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

/// Account stub written to `claudepot/accounts.export.json`. The shape
/// guarantees no secret-bearing field exists. Importer presents this
/// to the user as "the source machine had these accounts; re-login
/// here to use them."
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountStub {
    pub uuid: String,
    pub email: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub org_uuid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub org_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscription_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_limit_tier: Option<String>,
    /// Last verification outcome on the source. Carried so the user
    /// can prioritize re-login (drift accounts go first).
    pub verify_status: String,
}

/// Append the four claudepot-state files. Each is optional — the
/// caller may pass `None` for any source whose data isn't loaded.
pub fn append_claudepot_state(
    writer: &mut BundleWriter,
    accounts: &[AccountStub],
    protected_paths_json: Option<&[u8]>,
    preferences_json: Option<&[u8]>,
    artifact_lifecycle_json: Option<&[u8]>,
) -> Result<(), MigrateError> {
    // Account stubs always written, even when empty — the importer
    // checks for the file's presence to decide whether the bundle
    // carries account stubs at all.
    let bytes =
        serde_json::to_vec_pretty(accounts).map_err(|e| MigrateError::Serialize(e.to_string()))?;
    writer.append_bytes("claudepot/accounts.export.json", &bytes, 0o644)?;

    if let Some(p) = protected_paths_json {
        writer.append_bytes("claudepot/protected-paths.json", p, 0o644)?;
    }
    if let Some(p) = preferences_json {
        writer.append_bytes("claudepot/preferences.json", p, 0o644)?;
    }
    if let Some(p) = artifact_lifecycle_json {
        writer.append_bytes("claudepot/artifact-lifecycle.json", p, 0o644)?;
    }
    Ok(())
}

/// Build the account-stub list from a connected `AccountStore`.
/// Filters out any field that could shape an attacker's credential
/// recovery. Public so the orchestrator and CLI can both call it.
pub fn account_stubs_from_store(
    store: &crate::account::AccountStore,
) -> Result<Vec<AccountStub>, MigrateError> {
    let accounts = store
        .list()
        .map_err(|e| MigrateError::Serialize(format!("account list: {e}")))?;
    Ok(accounts
        .into_iter()
        .map(|a| AccountStub {
            uuid: a.uuid.to_string(),
            email: a.email,
            org_uuid: a.org_uuid,
            org_name: a.org_name,
            subscription_type: a.subscription_type,
            rate_limit_tier: a.rate_limit_tier,
            verify_status: a.verify_status,
        })
        .collect())
}

/// Read protected-paths JSON from the canonical store path. Returns
/// `Ok(None)` when no file exists yet.
pub fn read_protected_paths_bytes(data_dir: &Path) -> Result<Option<Vec<u8>>, MigrateError> {
    let p = crate::protected_paths::store_path(data_dir);
    if !p.exists() {
        return Ok(None);
    }
    fs::read(p).map(Some).map_err(MigrateError::from)
}

/// Read preferences JSON from `<data_dir>/preferences.json`. Returns
/// `Ok(None)` when no file exists.
pub fn read_preferences_bytes(data_dir: &Path) -> Result<Option<Vec<u8>>, MigrateError> {
    let p = data_dir.join("preferences.json");
    if !p.exists() {
        return Ok(None);
    }
    fs::read(p).map(Some).map_err(MigrateError::from)
}

/// Read artifact-lifecycle config from
/// `<data_dir>/artifact-lifecycle.json`. Returns `Ok(None)` when no
/// file exists.
pub fn read_artifact_lifecycle_bytes(data_dir: &Path) -> Result<Option<Vec<u8>>, MigrateError> {
    let p = data_dir.join("artifact-lifecycle.json");
    if !p.exists() {
        return Ok(None);
    }
    fs::read(p).map(Some).map_err(MigrateError::from)
}

/// Apply claudepot state on import. Per spec §16 Q2:
///   - Account stubs are LOG-ONLY. We do NOT auto-insert into the
///     account store. Returns the list so the caller can print it.
///   - Protected paths, preferences, artifact-lifecycle: written
///     to the target data dir, side-by-side if a file already
///     exists (`<name>.imported.<bundle_id_short>.json` so repeat
///     imports don't clobber prior review artifacts).
pub fn apply_claudepot_state(
    staging: &Path,
    data_dir: &Path,
    bundle_id: &str,
) -> Result<ClaudepotStateApplyOutcome, MigrateError> {
    let mut outcome = ClaudepotStateApplyOutcome::default();

    let accounts_path = staging.join("claudepot/accounts.export.json");
    if accounts_path.exists() {
        let bytes = fs::read(&accounts_path).map_err(MigrateError::from)?;
        let stubs: Vec<AccountStub> = serde_json::from_slice(&bytes)
            .map_err(|e| MigrateError::Serialize(format!("accounts.export.json: {e}")))?;
        outcome.accounts_listed = stubs;
        // Note: deliberately DO NOT call AccountStore::insert here.
        // Spec §16 Q2: log-only.
    }

    fs::create_dir_all(data_dir).map_err(MigrateError::from)?;

    for (rel_in_bundle, target_name) in [
        ("claudepot/protected-paths.json", "protected-paths.json"),
        ("claudepot/preferences.json", "preferences.json"),
        (
            "claudepot/artifact-lifecycle.json",
            "artifact-lifecycle.json",
        ),
    ] {
        let src = staging.join(rel_in_bundle);
        if !src.exists() {
            continue;
        }
        let target = data_dir.join(target_name);
        if !target.exists() {
            fs::copy(&src, &target).map_err(MigrateError::from)?;
            outcome.created.push(target.to_string_lossy().to_string());
        } else {
            // Side-by-side. Differing or identical, write `.imported`
            // so the user can audit before merging.
            let cur = fs::read(&target).map_err(MigrateError::from)?;
            let new = fs::read(&src).map_err(MigrateError::from)?;
            if cur == new {
                outcome
                    .skipped_identical
                    .push(target.to_string_lossy().to_string());
                continue;
            }
            let suffix = bundle_id.split('-').next().unwrap_or(bundle_id);
            let imported_name = target_name.replace(".json", &format!(".imported.{suffix}.json"));
            let imported = data_dir.join(imported_name);
            fs::copy(&src, &imported).map_err(MigrateError::from)?;
            outcome
                .side_by_side
                .push(imported.to_string_lossy().to_string());
        }
    }

    Ok(outcome)
}

/// Result of applying claudepot state on import. Caller surfaces it
/// to the user so the log-only accounts list isn't silent.
#[derive(Debug, Clone, Default)]
pub struct ClaudepotStateApplyOutcome {
    pub accounts_listed: Vec<AccountStub>,
    pub created: Vec<String>,
    pub side_by_side: Vec<String>,
    pub skipped_identical: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrate::bundle::{BundleReader, BundleWriter};
    use crate::migrate::manifest::{BundleManifest, ExportFlags, SCHEMA_VERSION};

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
            flags: ExportFlags {
                include_claudepot_state: true,
                ..Default::default()
            },
        }
    }

    #[test]
    fn account_stub_serialization_carries_no_credentials() {
        let s = AccountStub {
            uuid: "abc".to_string(),
            email: "x@y".to_string(),
            org_uuid: None,
            org_name: None,
            subscription_type: None,
            rate_limit_tier: None,
            verify_status: "ok".to_string(),
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("token"));
        assert!(!json.contains("credential"));
        assert!(!json.contains("sk-ant"));
    }

    #[test]
    fn append_writes_files_correctly() {
        let tmp = tempfile::tempdir().unwrap();
        let bundle_path = tmp.path().join("s.tar.zst");
        let mut w = BundleWriter::create(&bundle_path).unwrap();
        let stubs = vec![AccountStub {
            uuid: "abc".to_string(),
            email: "x@y".to_string(),
            org_uuid: None,
            org_name: None,
            subscription_type: None,
            rate_limit_tier: None,
            verify_status: "ok".to_string(),
        }];
        append_claudepot_state(
            &mut w,
            &stubs,
            Some(b"[]"),
            Some(br#"{"theme":"dark"}"#),
            None,
        )
        .unwrap();
        w.finalize(&fixture_manifest()).unwrap();
        let r = BundleReader::open(&bundle_path).unwrap();
        let bytes = r.read_entry("claudepot/accounts.export.json").unwrap();
        let back: Vec<AccountStub> = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].email, "x@y");
        let prefs = r.read_entry("claudepot/preferences.json").unwrap();
        assert_eq!(prefs, br#"{"theme":"dark"}"#);
        // artifact-lifecycle absent.
        let err = r.read_entry("claudepot/artifact-lifecycle.json");
        assert!(err.is_err());
    }

    #[test]
    fn apply_creates_files_when_target_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let staging = tmp.path().join("staging");
        fs::create_dir_all(staging.join("claudepot")).unwrap();
        fs::write(
            staging.join("claudepot/accounts.export.json"),
            r#"[{"uuid":"u","email":"x@y","verify_status":"ok"}]"#,
        )
        .unwrap();
        fs::write(
            staging.join("claudepot/preferences.json"),
            r#"{"theme":"dark"}"#,
        )
        .unwrap();

        let data_dir = tmp.path().join("data");
        let outcome = apply_claudepot_state(&staging, &data_dir, "test").unwrap();
        assert_eq!(outcome.accounts_listed.len(), 1);
        assert_eq!(outcome.accounts_listed[0].email, "x@y");
        assert!(data_dir.join("preferences.json").exists());
        assert_eq!(outcome.created.len(), 1);
    }

    #[test]
    fn apply_writes_side_by_side_when_target_differs() {
        let tmp = tempfile::tempdir().unwrap();
        let staging = tmp.path().join("staging");
        fs::create_dir_all(staging.join("claudepot")).unwrap();
        fs::write(staging.join("claudepot/accounts.export.json"), "[]").unwrap();
        fs::write(staging.join("claudepot/protected-paths.json"), r#"["/a"]"#).unwrap();
        let data_dir = tmp.path().join("data");
        fs::create_dir_all(&data_dir).unwrap();
        fs::write(data_dir.join("protected-paths.json"), r#"["/different"]"#).unwrap();

        let outcome = apply_claudepot_state(&staging, &data_dir, "test").unwrap();
        assert_eq!(outcome.side_by_side.len(), 1);
        // Suffix uses the test bundle id's first hyphen-segment.
        assert!(data_dir.join("protected-paths.imported.test.json").exists());
        // Original target untouched.
        assert_eq!(
            fs::read_to_string(data_dir.join("protected-paths.json")).unwrap(),
            r#"["/different"]"#
        );
    }

    #[test]
    fn apply_does_not_auto_insert_accounts_into_store() {
        // The contract: spec §16 Q2 says log-only, never auto-insert.
        // Pinned by the absence of an `AccountStore` parameter in
        // `apply_claudepot_state` — we only return the list.
        let tmp = tempfile::tempdir().unwrap();
        let staging = tmp.path().join("staging");
        fs::create_dir_all(staging.join("claudepot")).unwrap();
        fs::write(
            staging.join("claudepot/accounts.export.json"),
            r#"[{"uuid":"u","email":"x@y","verify_status":"drift"}]"#,
        )
        .unwrap();
        let data_dir = tmp.path().join("data");
        let outcome = apply_claudepot_state(&staging, &data_dir, "test").unwrap();
        assert_eq!(outcome.accounts_listed.len(), 1);
        // The function signature itself is the contract — there is no
        // way to inadvertently call AccountStore::insert from this
        // path because no store is passed.
    }
}
