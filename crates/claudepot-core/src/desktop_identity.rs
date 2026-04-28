//! Live Claude Desktop identity probing.
//!
//! The DB's `state.active_desktop` pointer is a cache of the last
//! switch Claudepot itself performed. It goes stale whenever the user
//! signs in or out of Claude Desktop directly. This module probes the
//! *live* Desktop session.
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
//! multiple accounts in the same org can produce the wrong email from
//! the fast path alone. UI surfaces that mutate disk or DB on the
//! identity's behalf MUST require a [`ProbeMethod::Decrypted`]
//! result — see [`VerifiedIdentity`].
//!
//! ## Slow path — decrypted + `/profile`
//! Reads the `oauth:tokenCache` ciphertext from `config.json`, pulls
//! the OS-specific key via `DesktopPlatform::safe_storage_secret`,
//! decrypts with [`crate::desktop_backend::crypto`], parses the
//! plaintext as [`DecryptedTokenCache`], and calls `/api/oauth/profile`
//! to confirm the identity. Returns [`ProbeMethod::Decrypted`] — the
//! only trust tier that constructs a [`VerifiedIdentity`].

// On non-macOS targets the slow-path token-cache decrypt machinery
// is gated out, so the import + the `fetch_profile` parameter that
// only feeds it land as warnings under `-D warnings`. Suppress just
// for those targets — macOS still enforces the lints.
#![cfg_attr(not(target_os = "macos"), allow(unused_imports, unused_variables))]

use crate::account::AccountStore;
use crate::desktop_backend::token_cache::DecryptedTokenCache;
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
    #[error("key retrieval failed: {0}")]
    Key(#[from] crate::desktop_backend::DesktopKeyError),
    #[error("decryption failed: {0}")]
    Decrypt(#[from] crate::desktop_backend::crypto::DecryptError),
    #[error("decrypted token cache is malformed: {0}")]
    TokenParse(String),
    #[error("/profile returned error: {0}")]
    ProfileFetch(String),
    #[error("slow-path identity probe unsupported on this platform")]
    Unsupported,
}

/// Type-level proof that a Desktop identity was obtained via the
/// authoritative decrypted+`/profile` path. Construction is private
/// to this module (via [`probe_live_identity`] on the slow path), so
/// any function taking `&VerifiedIdentity` is guaranteed its email
/// was server-confirmed.
///
/// Used by Phase 3 mutators (adopt, clear) so the type system
/// enforces the "no mutation on candidate identity" rule Codex D5-1
/// flagged as critical.
#[derive(Debug, Clone)]
pub struct VerifiedIdentity(LiveDesktopIdentity);

impl VerifiedIdentity {
    /// Public accessor — email is safe to forward; it came from the
    /// server's `/profile` response, not from anything the local
    /// config could forge.
    pub fn email(&self) -> &str {
        &self.0.email
    }
    pub fn org_uuid(&self) -> &str {
        &self.0.org_uuid
    }
    /// Unwrap — intentionally consumes. Rarely needed; most callers
    /// should use the accessors above.
    pub fn into_identity(self) -> LiveDesktopIdentity {
        self.0
    }

    /// In-crate-only constructor used by `desktop_service` tests to
    /// exercise mutators without running the full decrypt + /profile
    /// flow. Scoped `pub(crate)` so no downstream crate (Tauri, CLI)
    /// can bypass the Decrypted-only mutation guarantee — external
    /// callers must go through [`verify_live_identity`], which is
    /// the ONLY path that fetches an authoritative identity.
    ///
    /// Codex 2026-04-23 follow-up review (thread
    /// `019db814-a45b-7fa3-a280-80b1f20e1149`) flagged this as HIGH
    /// when it was `pub`.
    #[doc(hidden)]
    #[cfg(test)]
    pub(crate) fn from_live_for_testing(id: LiveDesktopIdentity) -> Self {
        Self(id)
    }
}

/// Probe the live Desktop session. Never mutates disk or DB.
///
/// See module docs for algorithm. Returns `Ok(None)` when the fast
/// path produced no candidate and the slow path either wasn't
/// requested (default) or couldn't resolve (signed out + strict).
///
/// Sync version — does not touch the network. Fast path only. Use
/// [`probe_live_identity_async`] when `strict=true` or when a
/// verified identity is needed.
pub fn probe_live_identity(
    platform: &dyn DesktopPlatform,
    store: &AccountStore,
    opts: ProbeOptions,
) -> Result<Option<LiveDesktopIdentity>, DesktopIdentityError> {
    if opts.strict {
        // Slow path requires async (keychain + HTTP). Surface a
        // helpful error so callers know to switch entry points.
        return Err(DesktopIdentityError::Unsupported);
    }
    let pieces = load_config(platform)?;
    fast_path_match(&pieces, store)
}

/// Async probe. On `strict=true` OR when the fast path can't resolve
/// (ambiguous / no registered match) AND `oauth:tokenCache` is
/// present, runs the authoritative slow path.
pub async fn probe_live_identity_async<F>(
    platform: &dyn DesktopPlatform,
    store: &AccountStore,
    opts: ProbeOptions,
    fetch_profile: &F,
) -> Result<Option<LiveDesktopIdentity>, DesktopIdentityError>
where
    F: ProfileFetcher + ?Sized,
{
    let pieces = load_config(platform)?;

    // Fast path first (unless strict).
    if !opts.strict {
        match fast_path_match(&pieces, store)? {
            Some(id) => return Ok(Some(id)),
            None => {
                // Fall through to slow path only if Desktop is
                // actually signed in.
                if pieces.token_cache_b64.is_none() {
                    return Ok(None);
                }
            }
        }
    }

    let Some(token_b64) = &pieces.token_cache_b64 else {
        return Err(DesktopIdentityError::NotSignedIn);
    };

    // Slow path: decrypt + /profile.
    let secret = platform.safe_storage_secret().await?;

    #[cfg(target_os = "macos")]
    let plaintext = crate::desktop_backend::crypto::macos::decrypt(token_b64, &secret)?;
    #[cfg(target_os = "windows")]
    let plaintext = crate::desktop_backend::crypto::windows::decrypt(token_b64, &secret)?;
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let _ = (token_b64, secret);
        return Err(DesktopIdentityError::Unsupported);
    }

    #[cfg(any(target_os = "macos", target_os = "windows"))]
    {
        let tc = DecryptedTokenCache::from_json(&plaintext)
            .map_err(|e| DesktopIdentityError::TokenParse(e.to_string()))?;
        let access = tc.pick_access_token().ok_or_else(|| {
            DesktopIdentityError::TokenParse("no access token found in decrypted bundles".into())
        })?;
        let prof = fetch_profile
            .fetch(access)
            .await
            .map_err(|e| DesktopIdentityError::ProfileFetch(e.to_string()))?;
        Ok(Some(LiveDesktopIdentity {
            email: prof.email,
            org_uuid: prof.org_uuid,
            probe_method: ProbeMethod::Decrypted,
        }))
    }
}

/// Convenience: full slow-path probe with the default Anthropic
/// profile endpoint. Returns a [`VerifiedIdentity`] on success so
/// mutators can use the type-level proof.
pub async fn verify_live_identity(
    platform: &dyn DesktopPlatform,
    store: &AccountStore,
) -> Result<Option<VerifiedIdentity>, DesktopIdentityError> {
    let fetcher = DefaultProfileFetcher;
    match probe_live_identity_async(platform, store, ProbeOptions { strict: true }, &fetcher)
        .await?
    {
        Some(id) if id.probe_method == ProbeMethod::Decrypted => Ok(Some(VerifiedIdentity(id))),
        Some(_) | None => Ok(None),
    }
}

/// Trait for fetching an OAuth profile — enables testing without
/// network. Mirrors the CLI-side `ProfileFetcher` pattern.
#[async_trait::async_trait]
pub trait ProfileFetcher: Send + Sync {
    async fn fetch(&self, access_token: &str) -> Result<ProfileResponse, String>;
}

pub struct ProfileResponse {
    pub email: String,
    pub org_uuid: String,
}

/// Production impl: hits the real `/api/oauth/profile` endpoint.
pub struct DefaultProfileFetcher;

#[async_trait::async_trait]
impl ProfileFetcher for DefaultProfileFetcher {
    async fn fetch(&self, access_token: &str) -> Result<ProfileResponse, String> {
        let p = crate::oauth::profile::fetch(access_token)
            .await
            .map_err(|e| e.to_string())?;
        Ok(ProfileResponse {
            email: p.email,
            org_uuid: p.org_uuid,
        })
    }
}

// Helpers split out so sync + async entry points share the same
// config-loading path.

struct ConfigPieces {
    cfg: serde_json::Value,
    org_uuids: Vec<Uuid>,
    token_cache_b64: Option<String>,
}

fn load_config(platform: &dyn DesktopPlatform) -> Result<ConfigPieces, DesktopIdentityError> {
    let data_dir = platform.data_dir().ok_or(DesktopIdentityError::NoDataDir)?;
    let cfg_path = data_dir.join("config.json");
    if !cfg_path.exists() {
        return Err(DesktopIdentityError::NoConfig);
    }
    let cfg = parse_config_json(&cfg_path)?;
    let org_uuids = extract_org_uuids(&cfg);
    let token_cache_b64 = cfg
        .get("oauth:tokenCache")
        .and_then(|v| v.as_str())
        .map(String::from);
    // Neither signal means Desktop is signed out.
    if org_uuids.is_empty() && token_cache_b64.is_none() {
        return Err(DesktopIdentityError::NotSignedIn);
    }
    Ok(ConfigPieces {
        cfg,
        org_uuids,
        token_cache_b64,
    })
}

fn fast_path_match(
    pieces: &ConfigPieces,
    store: &AccountStore,
) -> Result<Option<LiveDesktopIdentity>, DesktopIdentityError> {
    let _ = &pieces.cfg; // kept for future heuristics
                         // Collect candidates that are ALSO registered in the store.
                         // Ambiguous fast-path (≥2 accounts share the org) collapses via
                         // `find_by_org_uuid`'s unique-match contract.
    let mut matches: Vec<(Uuid, crate::account::Account)> = Vec::new();
    for org_uuid in &pieces.org_uuids {
        if let Ok(Some(acct)) = store.find_by_org_uuid(*org_uuid) {
            matches.push((*org_uuid, acct));
        }
    }
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
        let Some(tail) = k.rsplit(':').next() else {
            continue;
        };
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
        async fn safe_storage_secret(
            &self,
        ) -> Result<Vec<u8>, crate::desktop_backend::DesktopKeyError> {
            Err(crate::desktop_backend::DesktopKeyError::Unsupported)
        }
    }

    fn write_cfg(data_dir: &std::path::Path, body: serde_json::Value) {
        std::fs::create_dir_all(data_dir).unwrap();
        std::fs::write(data_dir.join("config.json"), body.to_string()).unwrap();
    }

    fn fake(data_dir: Option<std::path::PathBuf>) -> FakePlatform {
        FakePlatform { data_dir }
    }

    fn probe(
        p: &FakePlatform,
        s: &AccountStore,
        strict: bool,
    ) -> Result<Option<LiveDesktopIdentity>, DesktopIdentityError> {
        probe_live_identity(p, s, ProbeOptions { strict })
    }

    // --- extract_org_uuids (U1-U3) ---

    #[test]
    fn test_extract_org_uuids_finds_all_allowlist_keys() {
        let (u1, u2) = (Uuid::new_v4(), Uuid::new_v4());
        let cfg = serde_json::json!({
            "locale": "en-US",
            format!("dxt:allowlistEnabled:{}", u1): false,
            format!("dxt:allowlistCache:{}", u1): "djEw...",
            format!("dxt:allowlistLastUpdated:{}", u2): "2026-04-22T23:44:36.471Z",
            "oauth:tokenCache": "djEw...",
        });
        let out = extract_org_uuids(&cfg);
        assert_eq!(out.len(), 2);
        assert!(out.contains(&u1) && out.contains(&u2));
    }

    #[test]
    fn test_extract_org_uuids_empty_when_no_keys() {
        let cfg = serde_json::json!({ "locale": "en-US", "oauth:tokenCache": "djEw" });
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
        store
            .insert(&make_account("alice@example.com", Some(&org.to_string())))
            .unwrap();

        let tmp = tempfile::tempdir().unwrap();
        write_cfg(
            tmp.path(),
            serde_json::json!({
                format!("dxt:allowlistEnabled:{}", org): false,
                "oauth:tokenCache": "djEw...",
            }),
        );

        let id = probe(&fake(Some(tmp.path().to_path_buf())), &store, false)
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
                "oauth:tokenCache": "djEw...",
            }),
        );
        assert!(probe(&fake(Some(tmp.path().to_path_buf())), &store, false)
            .unwrap()
            .is_none());
    }

    #[test]
    fn test_probe_fast_path_multi_org_unresolved_is_none() {
        // Two DIFFERENT orgs in config, each uniquely matching one
        // account. Fast path still returns None — we can't pick one.
        let (store, _s) = store();
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        store
            .insert(&make_account("a@example.com", Some(&a.to_string())))
            .unwrap();
        store
            .insert(&make_account("b@example.com", Some(&b.to_string())))
            .unwrap();

        let tmp = tempfile::tempdir().unwrap();
        write_cfg(
            tmp.path(),
            serde_json::json!({
                format!("dxt:allowlistEnabled:{}", a): false,
                format!("dxt:allowlistEnabled:{}", b): false,
                "oauth:tokenCache": "djEw...",
            }),
        );
        assert!(probe(&fake(Some(tmp.path().to_path_buf())), &store, false)
            .unwrap()
            .is_none());
    }

    #[test]
    fn test_probe_empty_config_returns_not_signed_in() {
        // No allowlist UUIDs AND no oauth:tokenCache → Desktop is
        // signed out. Surface as NotSignedIn (load_config's guard).
        let (store, _s) = store();
        let tmp = tempfile::tempdir().unwrap();
        write_cfg(tmp.path(), serde_json::json!({ "locale": "en-US" }));
        let err = probe(&fake(Some(tmp.path().to_path_buf())), &store, false)
            .expect_err("signed-out must error");
        assert!(matches!(err, DesktopIdentityError::NotSignedIn));
    }

    #[test]
    fn test_probe_sync_rejects_strict() {
        // Sync probe cannot run the slow path (needs keychain +
        // HTTP). Callers must use probe_live_identity_async for
        // strict=true.
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
                "oauth:tokenCache": "djEw...",
            }),
        );
        let err = probe(&fake(Some(tmp.path().to_path_buf())), &store, true)
            .expect_err("sync probe cannot handle strict");
        assert!(matches!(err, DesktopIdentityError::Unsupported));
    }

    #[test]
    fn test_probe_no_config_json_returns_no_config() {
        let (store, _s) = store();
        let tmp = tempfile::tempdir().unwrap(); // no config.json
        let err = probe(&fake(Some(tmp.path().to_path_buf())), &store, false)
            .expect_err("missing config.json must error");
        assert!(matches!(err, DesktopIdentityError::NoConfig));
    }

    #[test]
    fn test_probe_no_data_dir_returns_no_data_dir() {
        let (store, _s) = store();
        let err =
            probe(&fake(None), &store, false).expect_err("platform without data_dir must error");
        assert!(matches!(err, DesktopIdentityError::NoDataDir));
    }

    // --- async slow-path tests (fake fetcher, no network) ---

    struct FixedFetcher {
        email: String,
        org_uuid: String,
    }

    #[async_trait::async_trait]
    impl super::ProfileFetcher for FixedFetcher {
        async fn fetch(&self, _: &str) -> Result<super::ProfileResponse, String> {
            Ok(super::ProfileResponse {
                email: self.email.clone(),
                org_uuid: self.org_uuid.clone(),
            })
        }
    }

    struct SlowPathFake {
        data_dir: std::path::PathBuf,
        secret: Vec<u8>,
    }

    #[async_trait::async_trait]
    impl DesktopPlatform for SlowPathFake {
        fn data_dir(&self) -> Option<std::path::PathBuf> {
            Some(self.data_dir.clone())
        }
        fn session_items(&self) -> &[&str] {
            &[]
        }
        async fn is_running(&self) -> bool {
            false
        }
        async fn quit(&self) -> Result<(), crate::error::DesktopSwapError> {
            Ok(())
        }
        async fn launch(&self) -> Result<(), crate::error::DesktopSwapError> {
            Ok(())
        }
        fn is_installed(&self) -> bool {
            true
        }
        async fn safe_storage_secret(
            &self,
        ) -> Result<Vec<u8>, crate::desktop_backend::DesktopKeyError> {
            Ok(self.secret.clone())
        }
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn test_slow_path_returns_decrypted_identity() {
        // Encrypt a synthetic tokenCache with a known secret, drop
        // it in a fake config.json, verify the async probe decrypts
        // + fetches profile + returns ProbeMethod::Decrypted.
        use aes::cipher::{block_padding::Pkcs7, BlockEncryptMut, KeyIvInit};
        type Enc = cbc::Encryptor<aes::Aes128>;

        let secret = b"test-keychain-secret".to_vec();
        // Shape must match DecryptedTokenCache's real layout: a
        // keyed map of bundle-keys → token envelopes.
        let tc_json = br#"{
            "uid-abc:org-def:https://api.anthropic.com:user:profile": {
                "token": "sk-ant-oat01-TESTTESTTEST",
                "refreshToken": "sk-ant-ort01-REFRESHREFRESH",
                "expiresAt": 1735689600000
            }
        }"#;
        let key = crate::desktop_backend::crypto::macos::derive_key(&secret);
        let iv = [b' '; 16];
        let ct = Enc::new_from_slices(&key, &iv)
            .unwrap()
            .encrypt_padded_vec_mut::<Pkcs7>(tc_json);
        let mut envelope = b"v10".to_vec();
        envelope.extend_from_slice(&ct);
        use base64::Engine as _;
        let token_b64 = base64::engine::general_purpose::STANDARD.encode(envelope);

        let tmp = tempfile::tempdir().unwrap();
        write_cfg(
            tmp.path(),
            serde_json::json!({
                "oauth:tokenCache": token_b64,
            }),
        );

        let (store, _s) = store();
        let platform = SlowPathFake {
            data_dir: tmp.path().to_path_buf(),
            secret,
        };
        let fetcher = FixedFetcher {
            email: "verified@example.com".into(),
            org_uuid: Uuid::new_v4().to_string(),
        };

        let id =
            probe_live_identity_async(&platform, &store, ProbeOptions { strict: true }, &fetcher)
                .await
                .unwrap()
                .expect("slow path returns verified identity");
        assert_eq!(id.email, "verified@example.com");
        assert_eq!(id.probe_method, ProbeMethod::Decrypted);
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
                "x@example.com",
                Some(&Uuid::new_v4().to_string()),
            ))
            .unwrap();

        let tmp = tempfile::tempdir().unwrap();
        write_cfg(
            tmp.path(),
            serde_json::json!({
                format!("dxt:allowlistEnabled:{}", org): false,
                "oauth:tokenCache": "djEw...",
            }),
        );
        assert!(probe(&fake(Some(tmp.path().to_path_buf())), &store, false)
            .unwrap()
            .is_none());
    }
}
