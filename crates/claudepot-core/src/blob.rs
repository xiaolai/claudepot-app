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
