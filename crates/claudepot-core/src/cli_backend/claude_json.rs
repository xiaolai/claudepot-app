//! Rewrite the `oauthAccount` block in `~/.claude.json`.
//!
//! CC's `claude auth status` and several internal UI elements read
//! `~/.claude.json` for the user-visible identity (email,
//! organization name, etc.) — NOT the keychain. If Claudepot swaps
//! the keychain but leaves `oauthAccount` stale, the user sees the
//! wrong account even though the underlying token is correct.
//!
//! This module plugs the gap by rewriting `oauthAccount` in-place
//! whenever the swap completes. Unknown fields (e.g.
//! `organizationRole`) are preserved — CC may write fields we don't
//! know about.

use crate::error::SwapError;
use crate::oauth::profile::Profile;
use serde_json::{json, Map, Value};
use std::fs;
use std::path::{Path, PathBuf};

/// Resolve `~/.claude.json`. Returns `None` if `$HOME` is unset.
pub fn default_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".claude.json"))
}

/// Read the current `oauthAccount` block from `path` for backup.
/// Returns `Ok(None)` if the file or the key doesn't exist (first
/// swap on a fresh install). Returns `Err` only on parse failure —
/// an I/O error on missing file is NOT propagated.
pub fn read_oauth_account(path: &Path) -> Result<Option<Value>, SwapError> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(SwapError::FileError(e)),
    };
    let root: Value = serde_json::from_str(&text)
        .map_err(|e| SwapError::CorruptBlob(format!(".claude.json: {e}")))?;
    Ok(root.get("oauthAccount").cloned())
}

/// Rewrite the `oauthAccount` block at `path` from a `Profile`.
///
/// Preserves fields the server didn't populate (e.g.
/// `organizationRole`, `workspaceRole`, `hasExtraUsageEnabled`) by
/// reading the existing block first and merging. Also preserves all
/// other top-level keys in the file — we only touch `oauthAccount`.
///
/// Writes atomically via tempfile + rename so a crash mid-write
/// can't corrupt the file.
pub fn update_oauth_account(path: &Path, profile: &Profile) -> Result<(), SwapError> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::from("{}"),
        Err(e) => return Err(SwapError::FileError(e)),
    };
    let mut root: Value = serde_json::from_str(&text)
        .map_err(|e| SwapError::CorruptBlob(format!(".claude.json: {e}")))?;

    // Start from the existing block (preserves unknown fields) and
    // overwrite what /profile gives us.
    let mut block: Map<String, Value> = root
        .get("oauthAccount")
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();

    block.insert("emailAddress".into(), json!(profile.email));
    block.insert("accountUuid".into(), json!(profile.account_uuid));
    block.insert("organizationUuid".into(), json!(profile.org_uuid));
    block.insert("organizationName".into(), json!(profile.org_name));
    if let Some(name) = &profile.display_name {
        block.insert("displayName".into(), json!(name));
    }

    if let Value::Object(m) = &mut root {
        m.insert("oauthAccount".into(), Value::Object(block));
    } else {
        // File wasn't an object (shouldn't happen if CC wrote it).
        // Replace with a fresh object containing only oauthAccount.
        let mut fresh = Map::new();
        fresh.insert("oauthAccount".into(), Value::Object(block));
        root = Value::Object(fresh);
    }

    write_atomic(path, &root)
}

/// Restore a previously-saved oauthAccount block — used by swap
/// rollback when a later step fails. `prior` being `None` means the
/// original file had no block; we remove the key entirely in that
/// case.
pub fn restore_oauth_account(path: &Path, prior: Option<&Value>) -> Result<(), SwapError> {
    let text = match fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::from("{}"),
        Err(e) => return Err(SwapError::FileError(e)),
    };
    let mut root: Value = serde_json::from_str(&text)
        .map_err(|e| SwapError::CorruptBlob(format!(".claude.json: {e}")))?;

    if let Value::Object(m) = &mut root {
        match prior {
            Some(v) => {
                m.insert("oauthAccount".into(), v.clone());
            }
            None => {
                m.remove("oauthAccount");
            }
        }
    }

    write_atomic(path, &root)
}

fn write_atomic(path: &Path, root: &Value) -> Result<(), SwapError> {
    let text = serde_json::to_string_pretty(root)
        .map_err(|e| SwapError::WriteFailed(format!("serialize .claude.json: {e}")))?;

    // Write tempfile next to the target so `rename` is atomic (same
    // filesystem). Using `.tmp-<pid>` suffix avoids collisions with
    // concurrent writers; if that concurrency actually happens, the
    // outer swap lock serializes us anyway.
    let parent = path
        .parent()
        .ok_or_else(|| SwapError::WriteFailed(".claude.json has no parent directory".into()))?;
    let tmp = parent.join(format!(".claude.json.tmp-{}", std::process::id()));
    fs::write(&tmp, text).map_err(SwapError::FileError)?;

    // Preserve original permissions if the file existed.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = fs::metadata(path) {
            let perms = fs::Permissions::from_mode(meta.permissions().mode());
            let _ = fs::set_permissions(&tmp, perms);
        } else {
            // Fresh file — mode 0600 (owner rw only).
            let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600));
        }
    }

    fs::rename(&tmp, path).map_err(SwapError::FileError)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn mk_profile() -> Profile {
        Profile {
            email: "new@example.com".into(),
            org_uuid: "org-new".into(),
            org_name: "New Org".into(),
            subscription_type: "max".into(),
            rate_limit_tier: None,
            account_uuid: "acct-new".into(),
            display_name: Some("New User".into()),
        }
    }

    #[test]
    fn test_update_rewrites_known_fields_preserves_unknown() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".claude.json");
        fs::write(
            &path,
            r#"{
                "oauthAccount": {
                    "emailAddress": "old@example.com",
                    "organizationRole": "admin",
                    "workspaceRole": null,
                    "billingType": "stripe_subscription"
                },
                "unrelatedKey": 42
            }"#,
        )
        .unwrap();

        update_oauth_account(&path, &mk_profile()).unwrap();

        let text = fs::read_to_string(&path).unwrap();
        let root: Value = serde_json::from_str(&text).unwrap();
        let block = &root["oauthAccount"];
        assert_eq!(block["emailAddress"], "new@example.com");
        assert_eq!(block["accountUuid"], "acct-new");
        assert_eq!(block["organizationName"], "New Org");
        // Preserved fields the server didn't provide:
        assert_eq!(block["organizationRole"], "admin");
        assert_eq!(block["workspaceRole"], Value::Null);
        assert_eq!(block["billingType"], "stripe_subscription");
        // Untouched top-level key:
        assert_eq!(root["unrelatedKey"], 42);
    }

    #[test]
    fn test_update_creates_block_if_missing() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".claude.json");
        fs::write(&path, r#"{"other": true}"#).unwrap();

        update_oauth_account(&path, &mk_profile()).unwrap();

        let text = fs::read_to_string(&path).unwrap();
        let root: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(root["oauthAccount"]["emailAddress"], "new@example.com");
        assert_eq!(root["other"], true);
    }

    #[test]
    fn test_update_handles_missing_file() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".claude.json");
        // File doesn't exist yet.
        update_oauth_account(&path, &mk_profile()).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        let root: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(root["oauthAccount"]["emailAddress"], "new@example.com");
    }

    #[test]
    fn test_read_oauth_account_missing_returns_none() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".claude.json");
        assert!(read_oauth_account(&path).unwrap().is_none());

        fs::write(&path, r#"{"other": 1}"#).unwrap();
        assert!(read_oauth_account(&path).unwrap().is_none());
    }

    #[test]
    fn test_read_oauth_account_returns_block() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".claude.json");
        fs::write(&path, r#"{"oauthAccount": {"emailAddress": "a@b.com"}}"#).unwrap();
        let block = read_oauth_account(&path).unwrap().unwrap();
        assert_eq!(block["emailAddress"], "a@b.com");
    }

    #[test]
    fn test_restore_replaces_block() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".claude.json");
        fs::write(
            &path,
            r#"{"oauthAccount": {"emailAddress": "current@x.com"}, "other": 1}"#,
        )
        .unwrap();

        let prior: Value =
            serde_json::from_str(r#"{"emailAddress": "original@x.com", "someField": true}"#)
                .unwrap();
        restore_oauth_account(&path, Some(&prior)).unwrap();

        let text = fs::read_to_string(&path).unwrap();
        let root: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(root["oauthAccount"]["emailAddress"], "original@x.com");
        assert_eq!(root["oauthAccount"]["someField"], true);
        assert_eq!(root["other"], 1);
    }

    #[test]
    fn test_restore_removes_block_when_prior_none() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join(".claude.json");
        fs::write(
            &path,
            r#"{"oauthAccount": {"emailAddress": "a@b.com"}, "other": 1}"#,
        )
        .unwrap();

        restore_oauth_account(&path, None).unwrap();

        let text = fs::read_to_string(&path).unwrap();
        let root: Value = serde_json::from_str(&text).unwrap();
        assert!(root.get("oauthAccount").is_none());
        assert_eq!(root["other"], 1);
    }
}
