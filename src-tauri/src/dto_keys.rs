//! Keys DTOs — ANTHROPIC_API_KEY and CLAUDE_CODE_OAUTH_TOKEN surface.
//!
//! The token itself NEVER crosses the IPC bridge on list / probe / add.
//! Only the redacted preview and metadata surface. `key_*_copy` is the
//! single deliberate exit — the user is explicitly asking to paste the
//! token into another tool, so we return the real value there.

use chrono::{DateTime, Utc};
use serde::Serialize;

/// One `ANTHROPIC_API_KEY` row on the Keys section. `account_uuid` is
/// a required soft reference — the linked account may still be removed
/// out of band, in which case the cross-database join leaves
/// `account_email = None` and the UI flags it as orphaned.
#[derive(Serialize, Clone)]
pub struct ApiKeySummaryDto {
    pub uuid: String,
    pub label: String,
    pub token_preview: String,
    pub account_uuid: String,
    pub account_email: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_probed_at: Option<DateTime<Utc>>,
    pub last_probe_status: Option<String>,
}

/// One `CLAUDE_CODE_OAUTH_TOKEN` row. `account_uuid` + `account_email`
/// are required at add-time (user picks the tag). `expires_at` is
/// derived from `created_at + OAUTH_TOKEN_VALIDITY_DAYS` — a proxy, not
/// ground truth; a 401 from the usage endpoint is the authoritative
/// expiry signal and flows through `last_probe_status`.
#[derive(Serialize, Clone)]
pub struct OauthTokenSummaryDto {
    pub uuid: String,
    pub label: String,
    pub token_preview: String,
    pub account_uuid: String,
    /// Email joined from `accounts.db` at read time. `None` when the
    /// linked account has been removed — the UI renders the raw uuid
    /// as a dimmed fallback in that case.
    pub account_email: Option<String>,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    /// Days between now and `expires_at`. Negative when already past
    /// the computed expiry; the UI chip flips red under 30 days.
    pub days_remaining: i64,
    pub last_probed_at: Option<DateTime<Utc>>,
    pub last_probe_status: Option<String>,
}
