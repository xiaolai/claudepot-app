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

    /// True when the blob parses but carries **no credentials at all** —
    /// both token fields empty.
    ///
    /// This is Claude Code's *cleared-credentials sentinel*. On some
    /// sign-out paths CC overwrites its keychain item rather than
    /// deleting it, leaving valid JSON with every key present, both
    /// tokens `""` and `expiresAt` zeroed, while `scopes` /
    /// `subscriptionType` / `rateLimitTier` survive intact.
    ///
    /// The danger is precisely that it **parses**. Without this
    /// predicate the sentinel flows down the ordinary path and reads as
    /// a live account whose token merely expired — so a state only a
    /// re-login can recover gets reported as a transient blip the user
    /// is told to wait out. An empty bearer can never authenticate and
    /// an empty refresh token can never be exchanged; callers must
    /// treat this as terminal.
    ///
    /// Deliberately does **not** test `expires_at == 0`, even though the
    /// observed sentinel zeroes it. A zeroed expiry is corroborating
    /// evidence, not the thing that makes the state unrecoverable, and
    /// requiring it would let a variant that keeps the old timestamp
    /// slip through as "just expired" — the exact misreport.
    ///
    /// Deliberately **does** require the access token to be empty too. A
    /// blob with a live access token and no refresh token authenticates
    /// right now, so calling it "signed out" would be wrong; that state
    /// is `can_refresh() == false` while this stays `false`.
    pub fn is_signed_out(&self) -> bool {
        self.claude_ai_oauth.access_token.is_empty()
            && self.claude_ai_oauth.refresh_token.is_empty()
    }

    /// True when there is a refresh token to spend.
    ///
    /// `false` is terminal: no exchange can succeed, so both the refresh
    /// itself *and the live-session gate that protects it* are pointless
    /// work on a known-dead state. That gate exists to avoid retiring a
    /// single-use token CC still holds in memory — with no token there
    /// is nothing to retire and nothing to protect, so a caller that
    /// consults the gate first converts a terminal state into a
    /// transient one.
    pub fn can_refresh(&self) -> bool {
        !self.claude_ai_oauth.refresh_token.is_empty()
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

    /// The sentinel PARSES — that is the whole defect it causes. This
    /// test states the premise every other assertion in this change
    /// rests on: `from_json` is not the guard, so a guard has to exist
    /// downstream of it.
    #[test]
    fn cleared_credentials_sentinel_parses_as_a_valid_blob() {
        let blob = CredentialBlob::from_json(&crate::testing::signed_out_blob_json())
            .expect("CC's cleared sentinel is valid JSON with every required key");
        assert!(blob.claude_ai_oauth.access_token.is_empty());
        assert!(blob.claude_ai_oauth.refresh_token.is_empty());
        assert_eq!(blob.claude_ai_oauth.expires_at, 0);
        // The surviving metadata is what makes it look like a real blob.
        assert_eq!(blob.claude_ai_oauth.scopes.len(), 2);
        assert_eq!(
            blob.claude_ai_oauth.subscription_type.as_deref(),
            Some("max")
        );
    }

    #[test]
    fn sentinel_is_signed_out_and_cannot_refresh() {
        let blob = CredentialBlob::from_json(&crate::testing::signed_out_blob_json()).unwrap();
        assert!(blob.is_signed_out());
        assert!(!blob.can_refresh());
        // It is also "expired" — which is exactly why it was mistaken
        // for a transient state. Being expired must not be the signal.
        assert!(blob.is_expired(300));
    }

    #[test]
    fn a_healthy_blob_is_neither_signed_out_nor_unrefreshable() {
        let blob = CredentialBlob::from_json(&crate::testing::fresh_blob_json()).unwrap();
        assert!(!blob.is_signed_out());
        assert!(blob.can_refresh());
    }

    /// An expired-but-refreshable blob is the case the sentinel check
    /// must never swallow: it is recoverable without any user action,
    /// and reporting it as "signed out" would send the user to a
    /// re-login they do not need.
    #[test]
    fn an_expired_blob_with_a_refresh_token_is_not_signed_out() {
        let blob = CredentialBlob::from_json(&crate::testing::expired_blob_json()).unwrap();
        assert!(blob.is_expired(0));
        assert!(!blob.is_signed_out());
        assert!(blob.can_refresh());
    }

    /// The asymmetric half-state: a live access token with no refresh
    /// token authenticates right now, so it is NOT signed out — but it
    /// has nothing to spend once the access token dies. The two
    /// predicates deliberately disagree here.
    #[test]
    fn live_access_token_without_a_refresh_token_is_not_signed_out() {
        let json = format!(
            r#"{{"claudeAiOauth":{{"accessToken":"sk-ant-oat01-live","refreshToken":"","expiresAt":{},"scopes":[]}}}}"#,
            chrono::Utc::now().timestamp_millis() + 3_600_000
        );
        let blob = CredentialBlob::from_json(&json).unwrap();
        assert!(!blob.is_signed_out(), "it can still authenticate");
        assert!(!blob.can_refresh(), "but nothing can renew it");
    }

    /// Guard against "just check `expires_at == 0`". A sentinel variant
    /// that kept the old timestamp would read as an ordinary expired
    /// blob under that rule, which is the misreport this whole change
    /// exists to remove.
    #[test]
    fn emptied_tokens_are_signed_out_even_with_a_live_looking_expiry() {
        let json = format!(
            r#"{{"claudeAiOauth":{{"accessToken":"","refreshToken":"","expiresAt":{},"scopes":[]}}}}"#,
            chrono::Utc::now().timestamp_millis() + 3_600_000
        );
        let blob = CredentialBlob::from_json(&json).unwrap();
        assert!(!blob.is_expired(0), "the expiry alone says 'healthy'");
        assert!(blob.is_signed_out(), "the tokens say otherwise");
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
