//! Frontend DTOs — what crosses the Tauri command boundary.
//!
//! We deliberately do NOT expose credential blobs, access tokens, or refresh
//! tokens to the webview. Only non-sensitive metadata leaves Rust.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Serialize)]
pub struct AccountSummary {
    pub uuid: String,
    pub email: String,
    pub org_name: Option<String>,
    pub subscription_type: Option<String>,
    pub is_cli_active: bool,
    pub is_desktop_active: bool,
    pub has_cli_credentials: bool,
    pub has_desktop_profile: bool,
    pub last_cli_switch: Option<DateTime<Utc>>,
    pub last_desktop_switch: Option<DateTime<Utc>>,
    /// "valid", "expired", "no credentials", "missing", "corrupt blob"
    pub token_status: String,
    pub token_remaining_mins: Option<i64>,
    /// True iff the stored blob actually exists and parses. Mirrors reality,
    /// not the DB flag. Used by the UI to gate the "Use CLI" button — the
    /// DB's has_cli_credentials can lie after external state changes.
    pub credentials_healthy: bool,
    /// Last persisted verification outcome: "never" | "ok" | "drift" |
    /// "rejected" | "network_error". Drives the drift badge in the UI.
    pub verify_status: String,
    /// When verify_status != "never", the actual email `/api/oauth/profile`
    /// returned for THIS slot. Equals `email` when ok; differs on drift.
    pub verified_email: Option<String>,
    /// ISO-8601 timestamp of the last verification pass.
    pub verified_at: Option<DateTime<Utc>>,
    /// Computed: verified_email is set AND differs from `email`. Handy
    /// for the GUI to avoid comparing strings itself.
    pub drift: bool,
}

impl From<&claudepot_core::account::Account> for AccountSummary {
    fn from(a: &claudepot_core::account::Account) -> Self {
        let health =
            claudepot_core::services::account_service::token_health(a.uuid, a.has_cli_credentials);
        // A stored blob is "healthy" if it exists and parses. Any other
        // status ("missing", "corrupt blob", "no credentials") means the
        // swap can't succeed — the UI should gate on this, not the DB flag.
        let credentials_healthy = health.status.starts_with("valid") || health.status == "expired";
        Self {
            uuid: a.uuid.to_string(),
            email: a.email.clone(),
            org_name: a.org_name.clone(),
            subscription_type: a.subscription_type.clone(),
            is_cli_active: a.is_cli_active,
            is_desktop_active: a.is_desktop_active,
            has_cli_credentials: a.has_cli_credentials,
            has_desktop_profile: a.has_desktop_profile,
            last_cli_switch: a.last_cli_switch,
            last_desktop_switch: a.last_desktop_switch,
            token_status: health.status,
            token_remaining_mins: health.remaining_mins,
            credentials_healthy,
            verify_status: a.verify_status.clone(),
            verified_email: a.verified_email.clone(),
            verified_at: a.verified_at,
            // Derive from verify_status, not `verified_email != email`.
            // update_verification() intentionally preserves
            // verified_email across rejected/network_error so history
            // isn't wiped by a blip — meaning a stored row where
            // verified_email still points at the old drift target but
            // verify_status has since moved to "network_error" would
            // spuriously paint as drift if we compared emails.
            drift: a.verify_status == "drift",
        }
    }
}

#[derive(Serialize)]
pub struct AppStatus {
    pub platform: String,
    pub arch: String,
    pub cli_active_email: Option<String>,
    pub desktop_active_email: Option<String>,
    pub desktop_installed: bool,
    pub data_dir: String,
    /// Absolute path of CC's config dir (`~/.claude`). The webview uses
    /// this to construct paths it hands straight back to
    /// `reveal_in_finder` — for example the session transcript at
    /// `<cc_config_dir>/projects/<slug>/<session_id>.jsonl`. Read-only
    /// metadata; shares code with `paths::claude_config_dir()` so the
    /// JS side never has to guess the home directory.
    pub cc_config_dir: String,
    pub account_count: usize,
}

#[derive(Serialize)]
pub struct RegisterOutcome {
    pub email: String,
    pub org_name: String,
    pub subscription_type: String,
}

#[derive(Serialize)]
pub struct RemoveOutcome {
    pub email: String,
    pub was_cli_active: bool,
    pub was_desktop_active: bool,
    pub had_desktop_profile: bool,
    pub warnings: Vec<String>,
}

/// A single usage window (utilization + reset time).
#[derive(Serialize, Clone)]
pub struct UsageWindowDto {
    // resets_at is optional: the server returns null for windows with
    // no activity yet. The frontend renders "\u2014" when missing.
    pub utilization: f64,
    pub resets_at: Option<String>, // RFC3339; null when the window has no reset yet
}

/// Extra-usage (monthly overage billing) info.
#[derive(Serialize, Clone)]
pub struct ExtraUsageDto {
    pub is_enabled: bool,
    pub monthly_limit: Option<f64>,
    pub used_credits: Option<f64>,
    /// Server-computed utilization percent (0–100). Preferred over a
    /// client-side `used / limit` recompute since the server knows
    /// about rollover, prorated credits, and grace adjustments the
    /// bare ratio misses.
    pub utilization: Option<f64>,
}

/// Per-account usage data. `None` fields mean the window is not active
/// for this subscription type, or no data is available.
#[derive(Serialize, Clone)]
pub struct AccountUsageDto {
    pub five_hour: Option<UsageWindowDto>,
    pub seven_day: Option<UsageWindowDto>,
    pub seven_day_opus: Option<UsageWindowDto>,
    pub seven_day_sonnet: Option<UsageWindowDto>,
    /// Usage attributed to third-party OAuth apps authorized against
    /// this account (IDEs, tools, etc). Null on plans that don't
    /// split this out; GUI renders render-if-nonzero.
    pub seven_day_oauth_apps: Option<UsageWindowDto>,
    /// Usage attributed to cowork / shared-seat pool. Null on
    /// personal plans; GUI renders render-if-nonzero.
    pub seven_day_cowork: Option<UsageWindowDto>,
    pub extra_usage: Option<ExtraUsageDto>,
}

impl AccountUsageDto {
    pub fn from_response(r: &claudepot_core::oauth::usage::UsageResponse) -> Self {
        let map_window = |w: &Option<claudepot_core::oauth::usage::UsageWindow>| {
            w.as_ref().map(|w| UsageWindowDto {
                utilization: w.utilization,
                resets_at: w.resets_at.as_ref().map(|t| t.to_rfc3339()),
            })
        };
        Self {
            five_hour: map_window(&r.five_hour),
            seven_day: map_window(&r.seven_day),
            seven_day_opus: map_window(&r.seven_day_opus),
            seven_day_sonnet: map_window(&r.seven_day_sonnet),
            seven_day_oauth_apps: map_window(&r.seven_day_oauth_apps),
            seven_day_cowork: map_window(&r.seven_day_cowork),
            extra_usage: r.extra_usage.as_ref().map(|e| ExtraUsageDto {
                is_enabled: e.is_enabled,
                monthly_limit: e.monthly_limit,
                used_credits: e.used_credits,
                utilization: e.utilization,
            }),
        }
    }
}

/// Per-account usage entry for the GUI. Carries enough state so the
/// UI can render an inline explanation when usage is unavailable
/// instead of silently dropping the account.
///
/// Status values (string-typed so the TS side can narrow on them):
///   - "ok"            — fresh data
///   - "stale"         — cached data, see `age_secs`
///   - "no_credentials"— account has no stored blob (shouldn't happen
///                       for has_cli_credentials=true, included for
///                       completeness)
///   - "expired"       — token past local expiry
///   - "rate_limited"  — on cooldown, no cache fallback
///   - "error"         — see `error_detail`
#[derive(Serialize, Clone)]
pub struct UsageEntryDto {
    pub status: String,
    /// Populated for "ok" and "stale".
    pub usage: Option<AccountUsageDto>,
    /// Seconds since the cache entry was written. Populated for "stale"
    /// and (approximately 0) for "ok".
    pub age_secs: Option<u64>,
    /// For "rate_limited": seconds until the cooldown clears.
    pub retry_after_secs: Option<u64>,
    /// For "error": a short technical string (e.g. "http 502",
    /// "timeout", "invalid json"). **This IS rendered verbatim** in
    /// the detail pane's UsageUnavailable block — keep the source
    /// (`UsageFetchError::FetchFailed`) free of tokens, URLs, or
    /// anything else that would be unsafe to show the user. The
    /// source is already scrubbed: it mirrors the error chain from
    /// the HTTP fetcher without request/response bodies.
    pub error_detail: Option<String>,
}

impl UsageEntryDto {
    pub fn from_outcome(
        outcome: claudepot_core::services::usage_cache::UsageOutcome,
    ) -> Self {
        use claudepot_core::services::usage_cache::UsageOutcome;
        match outcome {
            UsageOutcome::Fresh { response, age_secs } => Self {
                status: "ok".to_string(),
                usage: Some(AccountUsageDto::from_response(&response)),
                age_secs: Some(age_secs),
                retry_after_secs: None,
                error_detail: None,
            },
            UsageOutcome::Stale { response, age_secs } => Self {
                status: "stale".to_string(),
                usage: Some(AccountUsageDto::from_response(&response)),
                age_secs: Some(age_secs),
                retry_after_secs: None,
                error_detail: None,
            },
            UsageOutcome::NoCredentials => Self {
                status: "no_credentials".to_string(),
                usage: None,
                age_secs: None,
                retry_after_secs: None,
                error_detail: None,
            },
            UsageOutcome::Expired => Self {
                status: "expired".to_string(),
                usage: None,
                age_secs: None,
                retry_after_secs: None,
                error_detail: None,
            },
            UsageOutcome::RateLimited { retry_after_secs } => Self {
                status: "rate_limited".to_string(),
                usage: None,
                age_secs: None,
                retry_after_secs: Some(retry_after_secs),
                error_detail: None,
            },
            UsageOutcome::Error(msg) => Self {
                status: "error".to_string(),
                usage: None,
                age_secs: None,
                retry_after_secs: None,
                error_detail: Some(msg),
            },
        }
    }
}

/// Ground-truth "what is CC actually authenticated as right now".
///
/// Produced by the `current_cc_identity` Tauri command: reads CC's
/// shared credential slot, calls `/api/oauth/profile`, returns the
/// email the server confirms. The GUI's top-of-window truth strip
/// renders this directly — it's what `claude auth status` would print.
#[derive(Serialize)]
pub struct CcIdentity {
    /// The email `/api/oauth/profile` returned. `None` if CC has no
    /// stored blob or the blob is not parseable JSON.
    pub email: Option<String>,
    /// RFC3339 timestamp of when we ran the profile check. Lets the UI
    /// show "verified Ns ago" staleness.
    pub verified_at: chrono::DateTime<chrono::Utc>,
    /// Populated when CC has a blob but `/profile` failed — separate
    /// from `email=None` so the UI can distinguish "no CC credentials"
    /// from "couldn't reach the server" from "token revoked".
    pub error: Option<String>,
}

// ---------------------------------------------------------------------------
// Project DTOs — read-only surface (Step 2 of gui-rename plan)
// ---------------------------------------------------------------------------

/// Millisecond epoch helper. SystemTime isn't directly serde-friendly
/// for the JS heap; the webview wants a number it can pass to `new Date()`.
fn system_time_to_ms(t: Option<SystemTime>) -> Option<i64> {
    t.and_then(|st| st.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_millis() as i64)
}

#[derive(Serialize)]
pub struct ProjectInfoDto {
    pub sanitized_name: String,
    pub original_path: String,
    pub session_count: usize,
    pub memory_file_count: usize,
    pub total_size_bytes: u64,
    /// ms since epoch; null if the dir has never been modified (new).
    pub last_modified_ms: Option<i64>,
    pub is_orphan: bool,
    /// Could the source path be definitively stat'd? False for projects
    /// on unmounted removable volumes / offline shares / permission-
    /// denied ancestors. The GUI should surface this instead of showing
    /// a misleading "source missing" error.
    pub is_reachable: bool,
    /// CC project dir is effectively empty (no sessions, no memory).
    /// Useful for callers that want to show a distinct "abandoned"
    /// label from the standard "source deleted" orphan.
    pub is_empty: bool,
}

impl From<&claudepot_core::project_types::ProjectInfo> for ProjectInfoDto {
    fn from(p: &claudepot_core::project_types::ProjectInfo) -> Self {
        Self {
            sanitized_name: p.sanitized_name.clone(),
            original_path: p.original_path.clone(),
            session_count: p.session_count,
            memory_file_count: p.memory_file_count,
            total_size_bytes: p.total_size_bytes,
            last_modified_ms: system_time_to_ms(p.last_modified),
            is_orphan: p.is_orphan,
            is_reachable: p.is_reachable,
            is_empty: p.is_empty,
        }
    }
}

#[derive(Serialize)]
pub struct SessionInfoDto {
    pub session_id: String,
    pub file_size: u64,
    pub last_modified_ms: Option<i64>,
}

impl From<&claudepot_core::project_types::SessionInfo> for SessionInfoDto {
    fn from(s: &claudepot_core::project_types::SessionInfo) -> Self {
        Self {
            session_id: s.session_id.clone(),
            file_size: s.file_size,
            last_modified_ms: system_time_to_ms(s.last_modified),
        }
    }
}

#[derive(Serialize)]
pub struct ProjectDetailDto {
    pub info: ProjectInfoDto,
    pub sessions: Vec<SessionInfoDto>,
    pub memory_files: Vec<String>,
}

impl From<&claudepot_core::project_types::ProjectDetail> for ProjectDetailDto {
    fn from(p: &claudepot_core::project_types::ProjectDetail) -> Self {
        Self {
            info: ProjectInfoDto::from(&p.info),
            sessions: p.sessions.iter().map(SessionInfoDto::from).collect(),
            memory_files: p.memory_files.clone(),
        }
    }
}

/// What `project_clean_preview` returns — the list the user needs to
/// see before confirming a destructive clean. Mirrors
/// `CleanResult { orphans_found, unreachable_skipped, ... }` shape
/// but also ships the per-project list so the UI can render badges.
#[derive(Serialize)]
pub struct CleanPreviewDto {
    pub orphans: Vec<ProjectInfoDto>,
    pub orphans_found: usize,
    pub unreachable_skipped: usize,
    /// Sum of `total_size_bytes` across the candidate orphans. The UI
    /// displays this in the confirmation copy so users can judge the
    /// impact before pressing Confirm.
    pub total_bytes: u64,
    /// How many of the listed candidates have an authoritative source
    /// path that's in the user's protected-paths set. Their CC artifact
    /// dir will still be removed, but `~/.claude.json` and
    /// `history.jsonl` entries for those paths will be preserved. The
    /// confirmation modal uses this to disclose the carve-out before
    /// Confirm.
    pub protected_count: usize,
}

// Note: the post-clean result DTO lives in `ops::CleanResultSummary`
// because the clean op is event-driven (tokio task + op-progress
// events), so its result must attach to a `RunningOpInfo` rather than
// being returned inline from a Tauri command. Keep the clean preview
// DTO here (sync read-only call).

#[derive(Serialize)]
pub struct DryRunPlanDto {
    pub would_move_dir: bool,
    pub old_cc_dir: String,
    pub new_cc_dir: String,
    pub session_count: usize,
    pub cc_dir_size: u64,
    pub estimated_history_lines: usize,
    pub conflict: Option<String>,
    pub estimated_jsonl_files: usize,
    pub would_rewrite_claude_json: bool,
    pub would_move_memory_dir: bool,
    pub would_rewrite_project_settings: bool,
}

impl From<&claudepot_core::project_types::DryRunPlan> for DryRunPlanDto {
    fn from(p: &claudepot_core::project_types::DryRunPlan) -> Self {
        Self {
            would_move_dir: p.would_move_dir,
            old_cc_dir: p.old_cc_dir.clone(),
            new_cc_dir: p.new_cc_dir.clone(),
            session_count: p.session_count,
            cc_dir_size: p.cc_dir_size,
            estimated_history_lines: p.estimated_history_lines,
            conflict: p.conflict.clone(),
            estimated_jsonl_files: p.estimated_jsonl_files,
            would_rewrite_claude_json: p.would_rewrite_claude_json,
            would_move_memory_dir: p.would_move_memory_dir,
            would_rewrite_project_settings: p.would_rewrite_project_settings,
        }
    }
}

/// Per-status counts for the pending-journals banner. Pending +
/// stale are the two actionable classes; abandoned is filtered out.
/// `running` exists so the banner can suppress itself when the
/// op is already visible in the RunningOpStrip.
#[derive(Serialize)]
pub struct PendingJournalsSummaryDto {
    pub pending: usize,
    pub stale: usize,
    pub running: usize,
}

/// Inbound args from the webview for a dry-run move. Mirrors the
/// subset of `claudepot_core::project_types::MoveArgs` the UI controls;
/// config/snapshot paths are filled server-side from `claude_config_dir`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveArgsDto {
    pub old_path: String,
    pub new_path: String,
    #[serde(default)]
    pub no_move: bool,
    #[serde(default)]
    pub merge: bool,
    #[serde(default)]
    pub overwrite: bool,
    #[serde(default)]
    pub force: bool,
    #[serde(default)]
    pub ignore_pending_journals: bool,
    /// Monotonically increasing token for dry-run cancellation. When
    /// a newer token arrives during rapid typing, older in-flight
    /// calls bail out instead of returning stale plans. Present only
    /// on `project_move_dry_run`; other callers leave it None.
    #[serde(default)]
    pub cancel_token: Option<u64>,
}

// ---------------------------------------------------------------------------
// Repair DTOs
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct JournalFlagsDto {
    pub merge: bool,
    pub overwrite: bool,
    pub force: bool,
    pub no_move: bool,
}

impl From<&claudepot_core::project_journal::JournalFlags> for JournalFlagsDto {
    fn from(f: &claudepot_core::project_journal::JournalFlags) -> Self {
        Self {
            merge: f.merge,
            overwrite: f.overwrite,
            force: f.force,
            no_move: f.no_move,
        }
    }
}

#[derive(Serialize)]
pub struct JournalEntryDto {
    pub id: String,
    pub path: String,
    pub status: String,
    pub old_path: String,
    pub new_path: String,
    pub started_at: String,
    pub started_unix_secs: u64,
    pub phases_completed: Vec<String>,
    pub snapshot_paths: Vec<String>,
    pub last_error: Option<String>,
    pub flags: JournalFlagsDto,
}

impl From<&claudepot_core::project_repair::JournalEntry> for JournalEntryDto {
    fn from(e: &claudepot_core::project_repair::JournalEntry) -> Self {
        Self {
            id: e.id.clone(),
            path: e.path.to_string_lossy().to_string(),
            status: e.status.tag().to_string(),
            old_path: e.journal.old_path.clone(),
            new_path: e.journal.new_path.clone(),
            started_at: e.journal.started_at.clone(),
            started_unix_secs: e.journal.started_unix_secs,
            phases_completed: e.journal.phases_completed.clone(),
            snapshot_paths: e
                .journal
                .snapshot_paths
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect(),
            last_error: e.journal.last_error.clone(),
            flags: JournalFlagsDto::from(&e.journal.flags),
        }
    }
}

// ---------------------------------------------------------------------------
// Session move DTOs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OrphanedProjectDto {
    pub slug: String,
    pub cwd_from_transcript: Option<String>,
    pub session_count: usize,
    pub total_size_bytes: u64,
    pub suggested_adoption_target: Option<String>,
}

impl From<&claudepot_core::session_move::OrphanedProject> for OrphanedProjectDto {
    fn from(o: &claudepot_core::session_move::OrphanedProject) -> Self {
        Self {
            slug: o.slug.clone(),
            cwd_from_transcript: o
                .cwd_from_transcript
                .as_ref()
                .map(|p| p.display().to_string()),
            session_count: o.session_count,
            total_size_bytes: o.total_size_bytes,
            suggested_adoption_target: o
                .suggested_adoption_target
                .as_ref()
                .map(|p| p.display().to_string()),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MoveSessionReportDto {
    pub session_id: Option<String>,
    pub from_slug: String,
    pub to_slug: String,
    pub jsonl_lines_rewritten: usize,
    pub subagent_files_moved: usize,
    pub remote_agent_files_moved: usize,
    pub history_entries_moved: usize,
    pub history_entries_unmapped: usize,
    pub claude_json_pointers_cleared: u8,
    pub source_dir_removed: bool,
}

impl From<&claudepot_core::session_move::MoveSessionReport> for MoveSessionReportDto {
    fn from(r: &claudepot_core::session_move::MoveSessionReport) -> Self {
        Self {
            session_id: r.session_id.map(|s| s.to_string()),
            from_slug: r.from_slug.clone(),
            to_slug: r.to_slug.clone(),
            jsonl_lines_rewritten: r.jsonl_lines_rewritten,
            subagent_files_moved: r.subagent_files_moved,
            remote_agent_files_moved: r.remote_agent_files_moved,
            history_entries_moved: r.history_entries_moved,
            history_entries_unmapped: r.history_entries_unmapped,
            claude_json_pointers_cleared: r.claude_json_pointers_cleared,
            source_dir_removed: r.source_dir_removed,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdoptReportDto {
    pub sessions_attempted: usize,
    pub sessions_moved: usize,
    pub sessions_failed: Vec<AdoptFailureDto>,
    pub source_dir_removed: bool,
    pub per_session: Vec<MoveSessionReportDto>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AdoptFailureDto {
    pub session_id: String,
    pub error: String,
}

impl From<&claudepot_core::session_move::AdoptReport> for AdoptReportDto {
    fn from(r: &claudepot_core::session_move::AdoptReport) -> Self {
        Self {
            sessions_attempted: r.sessions_attempted,
            sessions_moved: r.sessions_moved,
            sessions_failed: r
                .sessions_failed
                .iter()
                .map(|(sid, msg)| AdoptFailureDto {
                    session_id: sid.to_string(),
                    error: msg.clone(),
                })
                .collect(),
            source_dir_removed: r.source_dir_removed,
            per_session: r.per_session.iter().map(MoveSessionReportDto::from).collect(),
        }
    }
}

/// One row in the protected-paths Settings list. `source` tells the
/// UI which badge to render (`default` | `user`).
#[derive(Serialize)]
pub struct ProtectedPathDto {
    pub path: String,
    /// Lowercase string: `"default"` or `"user"`. We don't expose the
    /// Rust enum variant names directly so the JS side doesn't need to
    /// keep its discriminant in lockstep with the core enum.
    pub source: String,
}

impl From<&claudepot_core::protected_paths::ProtectedPath> for ProtectedPathDto {
    fn from(p: &claudepot_core::protected_paths::ProtectedPath) -> Self {
        let source = match p.source {
            claudepot_core::protected_paths::PathSource::Default => "default",
            claudepot_core::protected_paths::PathSource::User => "user",
        };
        Self {
            path: p.path.clone(),
            source: source.to_string(),
        }
    }
}
