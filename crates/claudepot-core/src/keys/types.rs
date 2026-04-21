use chrono::{DateTime, Utc};
use uuid::Uuid;

/// A stored Anthropic API key (`sk-ant-api03-…`). The secret itself
/// lives in the OS keychain; this struct carries metadata only.
///
/// `account_uuid` is required at insert time — every key was created
/// under *some* account, and recording it makes the row findable by
/// account later. It's a soft reference (no FK into `accounts.db`),
/// so the row survives account removal — the cross-database join in
/// the DTO layer simply returns `account_email = null` for that case.
#[derive(Debug, Clone)]
pub struct ApiKey {
    pub uuid: Uuid,
    pub label: String,
    /// Redacted preview safe to render and log (`sk-ant-api03-Abc…xyz`).
    pub token_preview: String,
    pub account_uuid: Uuid,
    pub created_at: DateTime<Utc>,
    pub last_probed_at: Option<DateTime<Utc>>,
    pub last_probe_status: Option<String>,
}

/// A stored Claude Code OAuth token (`sk-ant-oat01-…`). Unlike API
/// keys, the account tag is required: the token inherits its account's
/// billing + quota, and the user picks the binding at add-time.
#[derive(Debug, Clone)]
pub struct OauthToken {
    pub uuid: Uuid,
    pub label: String,
    pub token_preview: String,
    pub account_uuid: Uuid,
    pub created_at: DateTime<Utc>,
    pub last_probed_at: Option<DateTime<Utc>>,
    pub last_probe_status: Option<String>,
}
