use serde::{Deserialize, Serialize};
use std::fmt;

/// The on-disk OAuth credential blob written by Claude Code CLI.
/// See reference.md Appendix A for the verified shape.
///
/// `Debug` is implemented manually so token bodies never appear in
/// `tracing::*`, panic messages, or `dbg!` output — the only thing
/// printed for token fields is the redacted length sentinel from
/// `OAuthCredentials::fmt`.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialBlob {
    pub claude_ai_oauth: OAuthCredentials,
}

impl fmt::Debug for CredentialBlob {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CredentialBlob")
            .field("claude_ai_oauth", &self.claude_ai_oauth)
            .finish()
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OAuthCredentials {
    /// Opaque 108-char token, prefix `sk-ant-oat01-`.
    pub access_token: String,

    /// Opaque 108-char token, prefix `sk-ant-ort01-`.
    pub refresh_token: String,

    /// Milliseconds since Unix epoch.
    pub expires_at: i64,

    /// Variable-length scope list. 5 elements in v2.1.92+ logins.
    /// Older blobs may have 2. Do not hardcode length.
    pub scopes: Vec<String>,

    /// "free", "pro", or "max".
    #[serde(default)]
    pub subscription_type: Option<String>,

    /// e.g. "default_claude_max_20x". May be empty string on older blobs.
    #[serde(default)]
    pub rate_limit_tier: Option<String>,
}

impl fmt::Debug for OAuthCredentials {
    /// Manual impl: `access_token` and `refresh_token` are redacted to
    /// `<redacted len=N>`. Per `.claude/rules/rust-conventions.md`, raw
    /// token bodies must never appear in any debug or log output.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OAuthCredentials")
            .field("access_token", &Redacted(self.access_token.len()))
            .field("refresh_token", &Redacted(self.refresh_token.len()))
            .field("expires_at", &self.expires_at)
            .field("scopes", &self.scopes)
            .field("subscription_type", &self.subscription_type)
            .field("rate_limit_tier", &self.rate_limit_tier)
            .finish()
    }
}

/// Tiny helper rendered as `<redacted len=N>` so debug output preserves
/// the token's length (useful for diagnosing truncated writes) without
/// ever exposing the body.
struct Redacted(usize);

impl fmt::Debug for Redacted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<redacted len={}>", self.0)
    }
}

impl CredentialBlob {
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    /// Check whether the access token is expired or will expire within
    /// the given margin (in seconds).
    pub fn is_expired(&self, margin_secs: i64) -> bool {
        let now_ms = chrono::Utc::now().timestamp_millis();
        self.claude_ai_oauth.expires_at < now_ms + (margin_secs * 1000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::sample_blob_json;

    #[test]
    fn test_blob_from_json_valid() {
        let json = sample_blob_json(9999999999999);
        let blob = CredentialBlob::from_json(&json).unwrap();
        assert_eq!(blob.claude_ai_oauth.access_token, "sk-ant-oat01-test");
        assert_eq!(blob.claude_ai_oauth.refresh_token, "sk-ant-ort01-test");
        assert_eq!(blob.claude_ai_oauth.expires_at, 9999999999999);
        assert_eq!(blob.claude_ai_oauth.scopes.len(), 2);
        assert_eq!(
            blob.claude_ai_oauth.subscription_type.as_deref(),
            Some("pro")
        );
        assert_eq!(
            blob.claude_ai_oauth.rate_limit_tier.as_deref(),
            Some("default_claude_pro")
        );
    }

    #[test]
    fn test_blob_from_json_minimal() {
        let json =
            r#"{"claudeAiOauth":{"accessToken":"t","refreshToken":"r","expiresAt":0,"scopes":[]}}"#;
        let blob = CredentialBlob::from_json(json).unwrap();
        assert!(blob.claude_ai_oauth.subscription_type.is_none());
        assert!(blob.claude_ai_oauth.rate_limit_tier.is_none());
    }

    #[test]
    fn test_blob_from_json_missing_required() {
        let json = r#"{"claudeAiOauth":{"refreshToken":"r","expiresAt":0,"scopes":[]}}"#;
        assert!(CredentialBlob::from_json(json).is_err());
    }

    #[test]
    fn test_blob_from_json_garbage() {
        assert!(CredentialBlob::from_json("not json").is_err());
        assert!(CredentialBlob::from_json("").is_err());
        assert!(CredentialBlob::from_json("{}").is_err());
    }

    #[test]
    fn test_blob_roundtrip() {
        let json = sample_blob_json(1234567890000);
        let blob = CredentialBlob::from_json(&json).unwrap();
        let serialized = blob.to_json().unwrap();
        let blob2 = CredentialBlob::from_json(&serialized).unwrap();
        assert_eq!(
            blob.claude_ai_oauth.access_token,
            blob2.claude_ai_oauth.access_token
        );
        assert_eq!(
            blob.claude_ai_oauth.expires_at,
            blob2.claude_ai_oauth.expires_at
        );
        assert_eq!(blob.claude_ai_oauth.scopes, blob2.claude_ai_oauth.scopes);
    }

    #[test]
    fn test_blob_is_expired_future() {
        let future = chrono::Utc::now().timestamp_millis() + 3_600_000; // +1h
        let json = sample_blob_json(future);
        let blob = CredentialBlob::from_json(&json).unwrap();
        assert!(!blob.is_expired(0));
    }

    #[test]
    fn test_blob_is_expired_past() {
        let past = chrono::Utc::now().timestamp_millis() - 3_600_000; // -1h
        let json = sample_blob_json(past);
        let blob = CredentialBlob::from_json(&json).unwrap();
        assert!(blob.is_expired(0));
    }

    #[test]
    fn test_blob_is_expired_within_margin() {
        let soon = chrono::Utc::now().timestamp_millis() + 30_000; // +30s
        let json = sample_blob_json(soon);
        let blob = CredentialBlob::from_json(&json).unwrap();
        assert!(!blob.is_expired(0)); // not expired without margin
        assert!(blob.is_expired(60)); // expired with 60s margin
    }

    /// Debug output must never reveal the access or refresh token body.
    /// `.claude/rules/rust-conventions.md` requires this — derived
    /// `Debug` would dump the raw `sk-ant-*` strings into any log line
    /// or panic that touches a CredentialBlob.
    #[test]
    fn test_blob_debug_redacts_tokens() {
        let json = r#"{"claudeAiOauth":{"accessToken":"sk-ant-oat01-SECRETLEAKVALUE","refreshToken":"sk-ant-ort01-OTHERLEAK","expiresAt":1,"scopes":["a"]}}"#;
        let blob = CredentialBlob::from_json(json).unwrap();
        let dbg = format!("{:?}", blob);
        assert!(
            !dbg.contains("sk-ant-oat01-SECRETLEAKVALUE"),
            "Debug must not include raw access token; got: {dbg}"
        );
        assert!(
            !dbg.contains("sk-ant-ort01-OTHERLEAK"),
            "Debug must not include raw refresh token; got: {dbg}"
        );
        assert!(
            !dbg.contains("SECRETLEAK"),
            "Debug must not include any partial token body; got: {dbg}"
        );
        // Length is preserved so operators can still tell something is
        // there (and how long).
        assert!(
            dbg.contains("len=28"),
            "Debug should record access-token length; got: {dbg}"
        );
        assert!(
            dbg.contains("len=22"),
            "Debug should record refresh-token length; got: {dbg}"
        );
        // Non-secret fields stay visible.
        assert!(dbg.contains("expires_at"));
        assert!(dbg.contains("scopes"));
    }

    #[test]
    fn test_blob_debug_with_alternate_formatter_also_redacts() {
        // `{:#?}` (pretty) and any other formatter route through the
        // same Debug impl — guard against a future regression that
        // reintroduces a bypass.
        let json = r#"{"claudeAiOauth":{"accessToken":"sk-ant-oat01-XYZ","refreshToken":"sk-ant-ort01-ABC","expiresAt":0,"scopes":[]}}"#;
        let blob = CredentialBlob::from_json(json).unwrap();
        let pretty = format!("{:#?}", blob);
        assert!(!pretty.contains("sk-ant-oat01-XYZ"));
        assert!(!pretty.contains("sk-ant-ort01-ABC"));
    }
}
