//! Parser for the plaintext produced by decrypting
//! `config.json → oauth:tokenCache`.
//!
//! # Real shape (empirically verified 2026-04-23 against live
//! Claude Desktop on macOS)
//!
//! ```json
//! {
//!   "<userId>:<orgUuid>:https://api.anthropic.com:<space-separated scopes>": {
//!     "token":        "sk-ant-oat01-…",
//!     "refreshToken": "sk-ant-ort01-…",
//!     "expiresAt":    1735689600000
//!   },
//!   "<userId>:<orgUuid>:https://api.anthropic.com:<scopes plus user:sessions:claude_code>": { … }
//! }
//! ```
//!
//! The top-level object is a **keyed map** of scope-bundles — not a
//! flat token record. Each key encodes the `userId`, `orgUuid`,
//! `apiHost`, and space-separated scope list; each value is a token
//! envelope. Multiple entries appear when the user has granted both
//! the base scope set and the extended `claude_code` scope set.
//!
//! We don't need to resolve which scope bundle "owns" the session —
//! any of the envelopes carries a valid access token for the same
//! user/org. We pick the first one (stable, because serde_json
//! preserves insertion order) and forward its `token` to `/profile`.
//!
//! # Key format
//!
//! ```text
//! <userUuid>:<orgUuid>:<apiHost>:<scopes>
//! ^^^^^^^^^^ ^^^^^^^^^^ ^^^^^^^^^ ^^^^^^^^
//! 36 chars   36 chars   (https://…) space-separated, may contain ':'
//! ```
//!
//! Parsing splits ONLY on the first three `:` characters to avoid
//! mangling the scope list (which contains internal `:` in scope
//! identifiers like `user:sessions:claude_code`).

use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Deserialize, Clone)]
pub struct TokenEnvelope {
    pub token: String,
    #[serde(default, rename = "refreshToken")]
    pub refresh_token: Option<String>,
    /// Epoch-millis, per Electron convention. Kept as i64 for forward
    /// compatibility with strings if Desktop ever changes shape.
    #[serde(default, rename = "expiresAt")]
    pub expires_at: Option<i64>,
}

/// The decrypted `oauth:tokenCache` body.
///
/// Stored as a `BTreeMap` keyed on the raw bundle key (the full
/// `userId:orgUuid:host:scopes` string) so callers can iterate all
/// entries in a stable order.
#[derive(Deserialize, Clone, Default)]
#[serde(transparent)]
pub struct DecryptedTokenCache(pub BTreeMap<String, TokenEnvelope>);

impl DecryptedTokenCache {
    pub fn from_json(plaintext: &[u8]) -> Result<Self, TokenParseError> {
        let cache: DecryptedTokenCache = serde_json::from_slice(plaintext)
            .map_err(|e| TokenParseError::Json(e.to_string()))?;
        if cache.0.is_empty() {
            return Err(TokenParseError::Empty);
        }
        Ok(cache)
    }

    /// Pick any token envelope with a non-empty access token. Callers
    /// only need one valid token to hit `/profile`; the server's
    /// response is what actually identifies the user.
    pub fn pick_access_token(&self) -> Option<&str> {
        self.0
            .values()
            .map(|e| e.token.as_str())
            .find(|t| !t.is_empty())
    }

    /// Parse a bundle key into its components. Returns `None` when
    /// the key shape doesn't match the expected four-segment form.
    /// Scopes are returned with internal `:` preserved (they contain
    /// colon-separated hierarchy like `user:sessions:claude_code`).
    ///
    /// The key format is `userUuid:orgUuid:URL:scopes` where URL
    /// carries its own `://` colon, so we can't naively splitn. We
    /// anchor on the first TWO `:` (userUuid/orgUuid boundary) and
    /// then on the `://` + next `:` to find the scope tail.
    pub fn parse_bundle_key(key: &str) -> Option<BundleKey<'_>> {
        let (user_uuid, rest) = key.split_once(':')?;
        let (org_uuid, rest) = rest.split_once(':')?;
        // rest starts with the URL. URL convention: `scheme://host[:port]`.
        // The scope list begins at the first `:` AFTER the `//` marker.
        let scheme_end = rest.find("://")? + 3;
        let host_and_scopes = &rest[scheme_end..];
        let scope_boundary = host_and_scopes.find(':')?;
        let api_host = &rest[..scheme_end + scope_boundary];
        let scopes = &host_and_scopes[scope_boundary + 1..];
        Some(BundleKey {
            user_uuid,
            org_uuid,
            api_host,
            scopes,
        })
    }

    /// Return the first parse-able bundle key's org UUID. The first
    /// two components of every key for a given signed-in session are
    /// identical (userId, orgUuid), so any entry yields the right
    /// answer. Falls back to None when no entry has a valid key.
    pub fn org_uuid(&self) -> Option<&str> {
        self.0.keys().find_map(|k| Self::parse_bundle_key(k).map(|p| p.org_uuid))
    }
}

pub struct BundleKey<'a> {
    pub user_uuid: &'a str,
    pub org_uuid: &'a str,
    pub api_host: &'a str,
    pub scopes: &'a str,
}

// `Debug` MUST redact tokens — .claude/rules/rust-conventions.md says
// NEVER log, print, or include access/refresh tokens in error output.
impl std::fmt::Debug for DecryptedTokenCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut d = f.debug_struct("DecryptedTokenCache");
        d.field("bundles", &self.0.len());
        for (key, env) in &self.0 {
            let short_key = match Self::parse_bundle_key(key) {
                Some(b) => format!("{}…@{} [{}]", &b.user_uuid[..8.min(b.user_uuid.len())], b.org_uuid, b.scopes),
                None => "<unparseable>".to_string(),
            };
            d.field(&short_key, &EnvelopeRedacted(env));
        }
        d.finish()
    }
}

struct EnvelopeRedacted<'a>(&'a TokenEnvelope);

impl std::fmt::Debug for EnvelopeRedacted<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenEnvelope")
            .field("token", &redact(&self.0.token))
            .field("refresh_token", &self.0.refresh_token.as_deref().map(redact))
            .field("expires_at", &self.0.expires_at)
            .finish()
    }
}

/// Truncate `sk-ant-oat01-ABC…XYZ` form. First 12 + last 3 of
/// the secret body; shape preserved so the class remains recognizable.
fn redact(token: &str) -> String {
    const PREFIX: usize = 12;
    const SUFFIX: usize = 3;
    if token.len() <= PREFIX + SUFFIX + 3 {
        return "***".to_string();
    }
    format!("{}…{}", &token[..PREFIX], &token[token.len() - SUFFIX..])
}

#[derive(Debug, thiserror::Error)]
pub enum TokenParseError {
    #[error("decrypted token cache is not valid JSON: {0}")]
    Json(String),
    #[error("decrypted token cache has no bundle entries")]
    Empty,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> &'static [u8] {
        br#"{
            "uid-1234-abcd:org-5678-efgh:https://api.anthropic.com:user:inference user:file_upload user:profile": {
                "token": "sk-ant-oat01-ACCESS-AAAAAAAAAAAA",
                "refreshToken": "sk-ant-ort01-REFRESH-BBBBBB",
                "expiresAt": 1735689600000
            },
            "uid-1234-abcd:org-5678-efgh:https://api.anthropic.com:user:inference user:file_upload user:profile user:sessions:claude_code": {
                "token": "sk-ant-oat01-ACCESS-CCCCCCCCCCCC",
                "refreshToken": "sk-ant-ort01-REFRESH-DDDDDD",
                "expiresAt": 1735689600000
            }
        }"#
    }

    #[test]
    fn test_parse_real_shape() {
        let t = DecryptedTokenCache::from_json(sample()).unwrap();
        assert_eq!(t.0.len(), 2);
    }

    #[test]
    fn test_parse_rejects_empty() {
        // Empty map = no valid session. Parser rejects rather than
        // silently succeed with a cache that has nothing to verify.
        assert!(matches!(
            DecryptedTokenCache::from_json(b"{}").unwrap_err(),
            TokenParseError::Empty
        ));
    }

    #[test]
    fn test_parse_rejects_garbage() {
        assert!(matches!(
            DecryptedTokenCache::from_json(b"not json").unwrap_err(),
            TokenParseError::Json(_)
        ));
    }

    #[test]
    fn test_pick_access_token() {
        let t = DecryptedTokenCache::from_json(sample()).unwrap();
        let tok = t.pick_access_token().expect("at least one token");
        assert!(tok.starts_with("sk-ant-oat01-ACCESS-"));
    }

    #[test]
    fn test_parse_bundle_key_shape() {
        let k = "uid-1234:org-5678:https://api.anthropic.com:user:inference user:sessions:claude_code";
        let b = DecryptedTokenCache::parse_bundle_key(k).unwrap();
        assert_eq!(b.user_uuid, "uid-1234");
        assert_eq!(b.org_uuid, "org-5678");
        assert_eq!(b.api_host, "https://api.anthropic.com");
        // Scopes retain their internal `:` — critical, because
        // scope identifiers like `user:sessions:claude_code` are
        // NOT key delimiters.
        assert_eq!(
            b.scopes,
            "user:inference user:sessions:claude_code"
        );
    }

    #[test]
    fn test_parse_bundle_key_rejects_malformed() {
        assert!(DecryptedTokenCache::parse_bundle_key("no-colons-here").is_none());
        assert!(DecryptedTokenCache::parse_bundle_key("only:two:parts").is_none());
    }

    #[test]
    fn test_org_uuid_from_cache() {
        let t = DecryptedTokenCache::from_json(sample()).unwrap();
        assert_eq!(t.org_uuid(), Some("org-5678-efgh"));
    }

    #[test]
    fn test_debug_redacts_all_tokens() {
        let t = DecryptedTokenCache::from_json(sample()).unwrap();
        let dbg = format!("{:?}", t);
        assert!(
            !dbg.contains("ACCESS-AAAAAAAAAAAA"),
            "full access token must not leak: {dbg}"
        );
        assert!(
            !dbg.contains("REFRESH-BBBBBB"),
            "full refresh token must not leak: {dbg}"
        );
    }
}
