//! Desktop-side identity DTOs.
//!
//! Mirrors `CcIdentity`'s never-throw contract — all failure modes
//! ride the `error` field so the UI can render them as visible banners
//! instead of dropped toasts.

use serde::Serialize;

/// How a Desktop identity was probed. Consumers that mutate disk or DB
/// on the identity's behalf MUST check this before acting — only
/// `Decrypted` is verified ground truth.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub enum DesktopProbeMethod {
    /// Fast path: the live `config.json`'s org UUID uniquely matched
    /// one registered account. Cheap, but NOT verified — users with
    /// multiple accounts in the same org produce wrong matches.
    OrgUuidCandidate,
    /// Slow path: decrypted `oauth:tokenCache` + successful `/profile`
    /// round-trip. Trusted. (Phase 2+ — returns `None` with an
    /// `Unimplemented` error in Phase 1.)
    Decrypted,
    /// Probe ran successfully but produced no identity (signed out,
    /// ambiguous org, or no registered account matches).
    None,
}

/// Result of `current_desktop_identity`. Mirrors `CcIdentity`'s
/// never-throw contract — all failure modes ride the `error` field so
/// the UI can render them as visible banners instead of dropped
/// toasts.
#[derive(Serialize, Clone, Debug)]
pub struct DesktopIdentity {
    /// Email of the signed-in Desktop account, when resolvable.
    pub email: Option<String>,
    /// Org UUID Desktop is signed in under, when resolvable.
    pub org_uuid: Option<String>,
    /// How the identity was obtained. UI trust level keys off this.
    pub probe_method: DesktopProbeMethod,
    /// RFC3339 of when the probe ran.
    pub verified_at: chrono::DateTime<chrono::Utc>,
    /// Populated on probe failure (data_dir missing, config malformed,
    /// slow-path not yet implemented, etc.).
    pub error: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
pub struct DesktopAdoptOutcome {
    pub account_email: String,
    pub captured_items: usize,
    pub size_bytes: u64,
}

#[derive(Serialize, Clone, Debug)]
pub struct DesktopClearOutcome {
    pub email: Option<String>,
    pub snapshot_kept: bool,
    pub items_deleted: usize,
}

#[derive(Serialize, Clone, Debug)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DesktopSyncOutcome {
    NoLive,
    Verified { email: String },
    AdoptionAvailable { email: String },
    Stranger { email: String },
    CandidateOnly { email: String },
}
