use serde::{Deserialize, Serialize};

/// The on-disk OAuth credential blob written by Claude Code CLI.
/// See reference.md Appendix A for the verified shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialBlob {
    pub claude_ai_oauth: OAuthCredentials,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
}
