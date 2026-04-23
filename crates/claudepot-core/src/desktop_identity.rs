//! Live Claude Desktop identity probing.
//!
//! The DB's `state.active_desktop` pointer is a cache of the last
//! switch Claudepot itself performed. It goes stale whenever the user
//! signs in or out of Claude Desktop directly. This module probes the
//! *live* Desktop session by reading `data_dir/config.json` and
//! (optionally, Phase 2+) decrypting `oauth:tokenCache`.
//!
//! # Probe methods
//!
//! ## Fast path — org-UUID candidate (no crypto)
//! `config.json` contains `dxt:allowlist[^:]*:<uuid>` keys whose UUID
//! part is the org UUID of the currently-signed-in account. If that
//! UUID matches **exactly one** registered account's `org_uuid`, the
//! match is returned as a *candidate* — [`ProbeMethod::OrgUuidCandidate`].
//!
//! **Important**: a candidate identity is NOT verified. Users with
//! multiple accounts in the same org (personal + team Max is common
//! in practice) can produce the wrong email from the fast path alone.
//! UI surfaces that mutate disk or DB on the identity's behalf MUST
//! require a [`ProbeMethod::Decrypted`] result or explicitly label
//! the affordance as "possible match — verify." See Codex review
//! 2026-04-23 D5-1 / D5-2.
//!
//! ## Slow path — decrypted + `/profile` (Phase 2+)
//! Stub in Phase 1 — returns `DesktopIdentityError::Unimplemented`.
//! Phase 2 wires the real decryption + profile round-trip.

use crate::account::AccountStore;
use crate::desktop_backend::DesktopPlatform;
use std::path::Path;
use uuid::Uuid;

/// Outcome of a live-identity probe. `probe_method` carries the trust
/// level — callers that mutate disk / DB MUST check it.
#[derive(Debug, Clone)]
pub struct LiveDesktopIdentity {
    pub email: String,
    pub org_uuid: String,
    pub probe_method: ProbeMethod,
}

/// How the identity was obtained. Determines trust.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeMethod {
    /// One registered account's `org_uuid` uniquely matches an org
    /// UUID in the live `config.json`. Cheap but NOT verified — do
    /// not drive mutation from this alone.
    OrgUuidCandidate,
    /// Decrypted `oauth:tokenCache` + successful `/profile` round-trip
    /// returned an email. Fully verified. Phase 2+.
    Decrypted,
}

/// Options controlling probe behavior.
#[derive(Debug, Clone, Copy, Default)]
pub struct ProbeOptions {
    /// Force the slow path even when a unique fast-path candidate
    /// exists. Phase 2+ — in Phase 1 this flag causes the probe to
    /// return `Unimplemented` instead of the candidate.
    pub strict: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum DesktopIdentityError {
    #[error("Desktop data_dir missing or unreadable: {0}")]
    DataDirUnreadable(String),
    #[error("Desktop data_dir not set for this platform")]
    NoDataDir,
    #[error("config.json is not present — Desktop has not been launched")]
    NoConfig,
    #[error("config.json is not valid JSON: {0}")]
    ConfigParse(String),
    #[error("no oauth:tokenCache present — Desktop is signed out")]
    NotSignedIn,
    #[error("slow-path identity probe not yet implemented (Phase 2)")]
    Unimplemented,
}

/// Probe the live Desktop session. Never mutates disk or DB.
///
/// See module docs for algorithm. Returns `Ok(None)` when the fast
/// path produced no candidate and the slow path is disabled (Phase 1)
/// or unable to resolve.
pub fn probe_live_identity(
    platform: &dyn DesktopPlatform,
    store: &AccountStore,
    opts: ProbeOptions,
) -> Result<Option<LiveDesktopIdentity>, DesktopIdentityError> {
    let data_dir = platform.data_dir().ok_or(DesktopIdentityError::NoDataDir)?;
    let cfg_path = data_dir.join("config.json");
    if !cfg_path.exists() {
        return Err(DesktopIdentityError::NoConfig);
    }

    let cfg = parse_config_json(&cfg_path)?;
    let org_uuids = extract_org_uuids(&cfg);

    // Empty list → either signed out, or this Desktop build uses a
    // layout that doesn't carry dxt:allowlist keys yet. Surface the
    // clearer of the two errors.
    if org_uuids.is_empty() {
        // Presence of oauth:tokenCache is a stronger signal of
        // "signed in, but we can't match via fast path."
        if cfg.get("oauth:tokenCache").is_some() {
            if opts.strict {
                return Err(DesktopIdentityError::Unimplemented);
            }
            // Can't verify without the slow path. Report None so
            // callers can treat this as "unknown identity."
            return Ok(None);
        }
        return Err(DesktopIdentityError::NotSignedIn);
    }

    // strict=true: skip the fast path entirely. Phase 2 wires the
    // slow path here; Phase 1 surfaces Unimplemented so callers know
    // not to promise verified identity.
    if opts.strict {
        return Err(DesktopIdentityError::Unimplemented);
    }

    // Collect candidates that are ALSO registered in the store.
    // Ambiguous fast-path (≥2 accounts share the org) collapses via
    // `find_by_org_uuid`'s unique-match contract.
    let mut matches: Vec<(Uuid, crate::account::Account)> = Vec::new();
    for org_uuid in &org_uuids {
        if let Ok(Some(acct)) = store.find_by_org_uuid(*org_uuid) {
            matches.push((*org_uuid, acct));
        }
    }

    // Two different orgs, each with one unique account → ambiguous
    // at the key-set level, even though each call was unique.
    // Refuse the candidate — force slow path.
    if matches.len() != 1 {
        return Ok(None);
    }

    let (org_uuid, account) = matches.into_iter().next().unwrap();
    Ok(Some(LiveDesktopIdentity {
        email: account.email,
        org_uuid: org_uuid.to_string(),
        probe_method: ProbeMethod::OrgUuidCandidate,
    }))
}

/// Extract org UUIDs from `dxt:allowlist[^:]*:<uuid>` keys. Case-
/// insensitive prefix "dxt:allowlist"; tail after the final ":" must
/// parse as a UUID. Malformed UUIDs are silently ignored.
///
/// Public for test-table-building in the identity probe tests.
pub fn extract_org_uuids(cfg: &serde_json::Value) -> Vec<Uuid> {
    let Some(obj) = cfg.as_object() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for k in obj.keys() {
        if !k.to_ascii_lowercase().starts_with("dxt:allowlist") {
            continue;
        }
        // The UUID is the substring after the LAST ':'. Using the last
        // `:` handles key variants like `dxt:allowlistEnabled:<uuid>`
        // and `dxt:allowlistLastUpdated:<uuid>` identically.
        let Some(tail) = k.rsplit(':').next() else { continue };
        if let Ok(uuid) = Uuid::parse_str(tail) {
            if !out.contains(&uuid) {
                out.push(uuid);
            }
        }
    }
    out
}

fn parse_config_json(path: &Path) -> Result<serde_json::Value, DesktopIdentityError> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| DesktopIdentityError::DataDirUnreadable(e.to_string()))?;
    serde_json::from_str::<serde_json::Value>(&raw)
        .map_err(|e| DesktopIdentityError::ConfigParse(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::account::{Account, AccountStore};
    use crate::desktop_backend::DesktopPlatform;
    use crate::error::DesktopSwapError;
    use chrono::Utc;

    fn store() -> (AccountStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("test.db");
        let store = AccountStore::open(&db).unwrap();
        (store, dir)
    }

    fn make_account(email: &str, org: Option<&str>) -> Account {
        Account {
            uuid: Uuid::new_v4(),
            email: email.to_string(),
            org_uuid: org.map(String::from),
            org_name: Some("Test Org".to_string()),
            subscription_type: Some("pro".to_string()),
            rate_limit_tier: Some("default".to_string()),
            created_at: Utc::now(),
            last_cli_switch: None,
            last_desktop_switch: None,
            has_cli_credentials: true,
            has_desktop_profile: false,
            is_cli_active: false,
            is_desktop_active: false,
            verified_email: None,
            verified_at: None,
            verify_status: "never".to_string(),
        }
    }

    struct FakePlatform {
        data_dir: Option<std::path::PathBuf>,
    }

    #[async_trait::async_trait]
    impl DesktopPlatform for FakePlatform {
        fn data_dir(&self) -> Option<std::path::PathBuf> {
            self.data_dir.clone()
        }
        fn session_items(&self) -> &[&str] {
            &[]
        }
        async fn is_running(&self) -> bool {
            false
        }
        async fn quit(&self) -> Result<(), DesktopSwapError> {
            Ok(())
        }
        async fn launch(&self) -> Result<(), DesktopSwapError> {
            Ok(())
        }
        fn is_installed(&self) -> bool {
            self.data_dir.is_some()
        }
    }

    fn write_cfg(data_dir: &std::path::Path, body: serde_json::Value) {
        std::fs::create_dir_all(data_dir).unwrap();
        std::fs::write(
            data_dir.join("config.json"),
            serde_json::to_string(&body).unwrap(),
        )
        .unwrap();
    }

    // --- extract_org_uuids unit tests (U1-U3) ---

    #[test]
    fn test_extract_org_uuids_finds_all_allowlist_keys() {
        let u1 = Uuid::new_v4();
        let u2 = Uuid::new_v4();
        let cfg = serde_json::json!({
            "locale": "en-US",
            format!("dxt:allowlistEnabled:{}", u1): false,
            format!("dxt:allowlistCache:{}", u1): "djEw...",
            format!("dxt:allowlistLastUpdated:{}", u2): "2026-04-22T23:44:36.471Z",
            "oauth:tokenCache": "djEw...",
        });
        let out = extract_org_uuids(&cfg);
        assert_eq!(out.len(), 2);
        assert!(out.contains(&u1));
        assert!(out.contains(&u2));
    }

    #[test]
    fn test_extract_org_uuids_empty_when_no_keys() {
        let cfg = serde_json::json!({
            "locale": "en-US",
            "oauth:tokenCache": "djEw...",
        });
        assert!(extract_org_uuids(&cfg).is_empty());
    }

    #[test]
    fn test_extract_org_uuids_ignores_malformed_uuid() {
        let cfg = serde_json::json!({
            "dxt:allowlistEnabled:not-a-uuid": false,
            "dxt:allowlistEnabled:deadbeef": false,
        });
        assert!(extract_org_uuids(&cfg).is_empty());
    }

    // --- probe_live_identity (U8-U11) ---

    #[test]
    fn test_probe_fast_path_unique_match_returns_candidate() {
        let (store, _s) = store();
        let org = Uuid::new_v4();
        let acct = make_account("alice@example.com", Some(&org.to_string()));
        store.insert(&acct).unwrap();

        let tmp = tempfile::tempdir().unwrap();
        write_cfg(
            tmp.path(),
            serde_json::json!({
                format!("dxt:allowlistEnabled:{}", org): false,
                "oauth:tokenCache": "djEw..."
            }),
        );
        let platform = FakePlatform {
            data_dir: Some(tmp.path().to_path_buf()),
        };

        let id = probe_live_identity(&platform, &store, ProbeOptions::default())
            .unwrap()
            .expect("unique match");
        assert_eq!(id.email, "alice@example.com");
        assert_eq!(id.org_uuid, org.to_string());
        assert_eq!(id.probe_method, ProbeMethod::OrgUuidCandidate);
    }

    #[test]
    fn test_probe_fast_path_ambiguous_returns_none() {
        // Two accounts in the same org → find_by_org_uuid returns
        // None → probe also returns None (forces slow path).
        let (store, _s) = store();
        let org = Uuid::new_v4();
        store
            .insert(&make_account("a@example.com", Some(&org.to_string())))
            .unwrap();
        store
            .insert(&make_account("b@example.com", Some(&org.to_string())))
            .unwrap();

        let tmp = tempfile::tempdir().unwrap();
        write_cfg(
            tmp.path(),
            serde_json::json!({
                format!("dxt:allowlistEnabled:{}", org): false,
                "oauth:tokenCache": "djEw..."
            }),
        );
        let platform = FakePlatform {
            data_dir: Some(tmp.path().to_path_buf()),
        };

        assert!(probe_live_identity(&platform, &store, ProbeOptions::default())
            .unwrap()
            .is_none());
    }

    #[test]
    fn test_probe_fast_path_multi_org_unresolved_is_none() {
        // Two DIFFERENT orgs in config, each uniquely matching one
        // account. Fast path still returns None — we can't pick one.
        let (store, _s) = store();
        let org_a = Uuid::new_v4();
        let org_b = Uuid::new_v4();
        store
            .insert(&make_account("a@example.com", Some(&org_a.to_string())))
            .unwrap();
        store
            .insert(&make_account("b@example.com", Some(&org_b.to_string())))
            .unwrap();

        let tmp = tempfile::tempdir().unwrap();
        write_cfg(
            tmp.path(),
            serde_json::json!({
                format!("dxt:allowlistEnabled:{}", org_a): false,
                format!("dxt:allowlistEnabled:{}", org_b): false,
                "oauth:tokenCache": "djEw..."
            }),
        );
        let platform = FakePlatform {
            data_dir: Some(tmp.path().to_path_buf()),
        };

        assert!(probe_live_identity(&platform, &store, ProbeOptions::default())
            .unwrap()
            .is_none());
    }

    #[test]
    fn test_probe_no_oauth_token_returns_not_signed_in() {
        let (store, _s) = store();
        let tmp = tempfile::tempdir().unwrap();
        write_cfg(tmp.path(), serde_json::json!({ "locale": "en-US" }));
        let platform = FakePlatform {
            data_dir: Some(tmp.path().to_path_buf()),
        };

        let err = probe_live_identity(&platform, &store, ProbeOptions::default())
            .expect_err("should error on signed-out");
        assert!(matches!(err, DesktopIdentityError::NotSignedIn));
    }

    #[test]
    fn test_probe_strict_mode_returns_unimplemented_in_phase_1() {
        let (store, _s) = store();
        let org = Uuid::new_v4();
        store
            .insert(&make_account("a@example.com", Some(&org.to_string())))
            .unwrap();

        let tmp = tempfile::tempdir().unwrap();
        write_cfg(
            tmp.path(),
            serde_json::json!({
                format!("dxt:allowlistEnabled:{}", org): false,
                "oauth:tokenCache": "djEw..."
            }),
        );
        let platform = FakePlatform {
            data_dir: Some(tmp.path().to_path_buf()),
        };

        let err = probe_live_identity(
            &platform,
            &store,
            ProbeOptions { strict: true },
        )
        .expect_err("strict must fail until Phase 2 crypto lands");
        assert!(matches!(err, DesktopIdentityError::Unimplemented));
    }

    #[test]
    fn test_probe_no_config_json_returns_no_config() {
        let (store, _s) = store();
        let tmp = tempfile::tempdir().unwrap();
        // deliberately DON'T write config.json
        let platform = FakePlatform {
            data_dir: Some(tmp.path().to_path_buf()),
        };
        let err = probe_live_identity(&platform, &store, ProbeOptions::default())
            .expect_err("should error when config.json missing");
        assert!(matches!(err, DesktopIdentityError::NoConfig));
    }

    #[test]
    fn test_probe_no_data_dir_returns_no_data_dir() {
        let (store, _s) = store();
        let platform = FakePlatform { data_dir: None };
        let err = probe_live_identity(&platform, &store, ProbeOptions::default())
            .expect_err("platform without data_dir should error");
        assert!(matches!(err, DesktopIdentityError::NoDataDir));
    }

    #[test]
    fn test_probe_signed_in_but_no_registered_match_returns_none() {
        // Desktop is signed in (org keys + oauth token present), but
        // no Claudepot account has matching org_uuid. Fast path has
        // no candidate; slow path disabled in Phase 1. Result: None,
        // not Err — the call was well-formed, we just don't know who.
        let (store, _s) = store();
        let org = Uuid::new_v4();
        store
            .insert(&make_account(
                "unrelated@example.com",
                Some(&Uuid::new_v4().to_string()),
            ))
            .unwrap();

        let tmp = tempfile::tempdir().unwrap();
        write_cfg(
            tmp.path(),
            serde_json::json!({
                format!("dxt:allowlistEnabled:{}", org): false,
                "oauth:tokenCache": "djEw..."
            }),
        );
        let platform = FakePlatform {
            data_dir: Some(tmp.path().to_path_buf()),
        };

        assert!(probe_live_identity(&platform, &store, ProbeOptions::default())
            .unwrap()
            .is_none());
    }
}
