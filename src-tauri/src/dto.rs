//! Frontend DTOs — what crosses the Tauri command boundary.
//!
//! We deliberately do NOT expose credential blobs, access tokens, or refresh
//! tokens to the webview. Only non-sensitive metadata leaves Rust.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Serialize, Clone)]
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
    /// Monthly cap and spend are returned in MINOR currency units
    /// (e.g. pence for GBP, cents for USD). Frontend divides by 100
    /// before rendering.
    pub monthly_limit: Option<f64>,
    pub used_credits: Option<f64>,
    /// Server-computed utilization percent (0–100). Preferred over a
    /// client-side `used / limit` recompute since the server knows
    /// about rollover, prorated credits, and grace adjustments the
    /// bare ratio misses.
    pub utilization: Option<f64>,
    /// ISO 4217 currency code ("USD", "GBP", …). None on older
    /// responses; frontend falls back to USD.
    pub currency: Option<String>,
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
                currency: e.currency.clone(),
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

// ---------------------------------------------------------------------------
// Session index — list and transcript reader for the Sessions tab.
// Mirrors claudepot_core::session::*.
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct TokenUsageDto {
    pub input: u64,
    pub output: u64,
    pub cache_creation: u64,
    pub cache_read: u64,
    pub total: u64,
}

impl From<&claudepot_core::session::TokenUsage> for TokenUsageDto {
    fn from(t: &claudepot_core::session::TokenUsage) -> Self {
        Self {
            input: t.input,
            output: t.output,
            cache_creation: t.cache_creation,
            cache_read: t.cache_read,
            total: t.total(),
        }
    }
}

#[derive(Serialize)]
pub struct SessionRowDto {
    pub session_id: String,
    pub slug: String,
    pub file_path: String,
    pub file_size_bytes: u64,
    pub last_modified_ms: Option<i64>,
    pub project_path: String,
    pub project_from_transcript: bool,
    pub first_ts: Option<DateTime<Utc>>,
    pub last_ts: Option<DateTime<Utc>>,
    pub event_count: usize,
    pub message_count: usize,
    pub user_message_count: usize,
    pub assistant_message_count: usize,
    pub first_user_prompt: Option<String>,
    pub models: Vec<String>,
    pub tokens: TokenUsageDto,
    pub git_branch: Option<String>,
    pub cc_version: Option<String>,
    pub display_slug: Option<String>,
    pub has_error: bool,
    pub is_sidechain: bool,
}

impl From<&claudepot_core::session::SessionRow> for SessionRowDto {
    fn from(r: &claudepot_core::session::SessionRow) -> Self {
        Self {
            session_id: r.session_id.clone(),
            slug: r.slug.clone(),
            file_path: r.file_path.to_string_lossy().to_string(),
            file_size_bytes: r.file_size_bytes,
            last_modified_ms: system_time_to_ms(r.last_modified),
            project_path: r.project_path.clone(),
            project_from_transcript: r.project_from_transcript,
            first_ts: r.first_ts,
            last_ts: r.last_ts,
            event_count: r.event_count,
            message_count: r.message_count,
            user_message_count: r.user_message_count,
            assistant_message_count: r.assistant_message_count,
            first_user_prompt: r.first_user_prompt.clone(),
            models: r.models.clone(),
            tokens: TokenUsageDto::from(&r.tokens),
            git_branch: r.git_branch.clone(),
            cc_version: r.cc_version.clone(),
            display_slug: r.display_slug.clone(),
            has_error: r.has_error,
            is_sidechain: r.is_sidechain,
        }
    }
}

#[derive(Serialize)]
#[serde(tag = "kind")]
pub enum SessionEventDto {
    #[serde(rename = "userText")]
    UserText {
        ts: Option<DateTime<Utc>>,
        uuid: Option<String>,
        text: String,
    },
    #[serde(rename = "userToolResult")]
    UserToolResult {
        ts: Option<DateTime<Utc>>,
        uuid: Option<String>,
        tool_use_id: String,
        content: String,
        is_error: bool,
    },
    #[serde(rename = "assistantText")]
    AssistantText {
        ts: Option<DateTime<Utc>>,
        uuid: Option<String>,
        model: Option<String>,
        text: String,
        usage: Option<TokenUsageDto>,
        stop_reason: Option<String>,
    },
    #[serde(rename = "assistantToolUse")]
    AssistantToolUse {
        ts: Option<DateTime<Utc>>,
        uuid: Option<String>,
        model: Option<String>,
        tool_name: String,
        tool_use_id: String,
        input_preview: String,
    },
    #[serde(rename = "assistantThinking")]
    AssistantThinking {
        ts: Option<DateTime<Utc>>,
        uuid: Option<String>,
        text: String,
    },
    #[serde(rename = "summary")]
    Summary {
        ts: Option<DateTime<Utc>>,
        uuid: Option<String>,
        text: String,
    },
    #[serde(rename = "system")]
    System {
        ts: Option<DateTime<Utc>>,
        uuid: Option<String>,
        subtype: Option<String>,
        detail: String,
    },
    #[serde(rename = "attachment")]
    Attachment {
        ts: Option<DateTime<Utc>>,
        uuid: Option<String>,
        name: Option<String>,
        mime: Option<String>,
    },
    #[serde(rename = "fileSnapshot")]
    FileHistorySnapshot {
        ts: Option<DateTime<Utc>>,
        uuid: Option<String>,
        file_count: usize,
    },
    #[serde(rename = "other")]
    Other {
        ts: Option<DateTime<Utc>>,
        uuid: Option<String>,
        raw_type: String,
    },
    #[serde(rename = "malformed")]
    Malformed {
        line_number: usize,
        error: String,
        preview: String,
    },
}

impl From<&claudepot_core::session::SessionEvent> for SessionEventDto {
    fn from(e: &claudepot_core::session::SessionEvent) -> Self {
        use claudepot_core::session::SessionEvent as E;
        match e {
            E::UserText { ts, uuid, text } => Self::UserText {
                ts: *ts,
                uuid: uuid.clone(),
                text: text.clone(),
            },
            E::UserToolResult {
                ts,
                uuid,
                tool_use_id,
                content,
                is_error,
            } => Self::UserToolResult {
                ts: *ts,
                uuid: uuid.clone(),
                tool_use_id: tool_use_id.clone(),
                content: content.clone(),
                is_error: *is_error,
            },
            E::AssistantText {
                ts,
                uuid,
                model,
                text,
                usage,
                stop_reason,
            } => Self::AssistantText {
                ts: *ts,
                uuid: uuid.clone(),
                model: model.clone(),
                text: text.clone(),
                usage: usage.as_ref().map(TokenUsageDto::from),
                stop_reason: stop_reason.clone(),
            },
            E::AssistantToolUse {
                ts,
                uuid,
                model,
                tool_name,
                tool_use_id,
                input_preview,
            } => Self::AssistantToolUse {
                ts: *ts,
                uuid: uuid.clone(),
                model: model.clone(),
                tool_name: tool_name.clone(),
                tool_use_id: tool_use_id.clone(),
                input_preview: input_preview.clone(),
            },
            E::AssistantThinking { ts, uuid, text } => Self::AssistantThinking {
                ts: *ts,
                uuid: uuid.clone(),
                text: text.clone(),
            },
            E::Summary { ts, uuid, text } => Self::Summary {
                ts: *ts,
                uuid: uuid.clone(),
                text: text.clone(),
            },
            E::System {
                ts,
                uuid,
                subtype,
                detail,
            } => Self::System {
                ts: *ts,
                uuid: uuid.clone(),
                subtype: subtype.clone(),
                detail: detail.clone(),
            },
            E::Attachment {
                ts,
                uuid,
                name,
                mime,
            } => Self::Attachment {
                ts: *ts,
                uuid: uuid.clone(),
                name: name.clone(),
                mime: mime.clone(),
            },
            E::FileHistorySnapshot {
                ts,
                uuid,
                file_count,
            } => Self::FileHistorySnapshot {
                ts: *ts,
                uuid: uuid.clone(),
                file_count: *file_count,
            },
            E::Other {
                ts,
                uuid,
                raw_type,
            } => Self::Other {
                ts: *ts,
                uuid: uuid.clone(),
                raw_type: raw_type.clone(),
            },
            E::Malformed {
                line_number,
                error,
                preview,
            } => Self::Malformed {
                line_number: *line_number,
                error: error.clone(),
                preview: preview.clone(),
            },
        }
    }
}

#[derive(Serialize)]
pub struct SessionDetailDto {
    pub row: SessionRowDto,
    pub events: Vec<SessionEventDto>,
}

impl From<&claudepot_core::session::SessionDetail> for SessionDetailDto {
    fn from(d: &claudepot_core::session::SessionDetail) -> Self {
        Self {
            row: SessionRowDto::from(&d.row),
            events: d.events.iter().map(SessionEventDto::from).collect(),
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

// ---------------------------------------------------------------------------
// Session debugger DTOs (Tier 1-3 claude-devtools port)
//
// Purpose: pin the webview-facing JSON contract so that changes to core
// serde shapes (new fields, renamed enums, etc.) cannot implicitly flip
// the JS bindings. Each DTO is a structural clone of the core type, with
// an explicit `From<&CoreType>` conversion. Reuses the existing
// `TokenUsageDto` defined above for consistency with the rest of the
// session surface.
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct ChunkMetricsDto {
    pub duration_ms: i64,
    pub tokens: TokenUsageDto,
    pub message_count: usize,
    pub tool_call_count: usize,
    pub thinking_count: usize,
}

impl From<&claudepot_core::session_chunks::ChunkMetrics> for ChunkMetricsDto {
    fn from(m: &claudepot_core::session_chunks::ChunkMetrics) -> Self {
        Self {
            duration_ms: m.duration_ms,
            tokens: (&m.tokens).into(),
            message_count: m.message_count,
            tool_call_count: m.tool_call_count,
            thinking_count: m.thinking_count,
        }
    }
}

#[derive(Serialize)]
pub struct LinkedToolDto {
    pub tool_use_id: String,
    pub tool_name: String,
    pub model: Option<String>,
    pub call_ts: Option<DateTime<Utc>>,
    pub input_preview: String,
    pub result_ts: Option<DateTime<Utc>>,
    pub result_content: Option<String>,
    pub is_error: bool,
    pub duration_ms: Option<i64>,
    pub call_index: usize,
    pub result_index: Option<usize>,
}

impl From<&claudepot_core::session_tool_link::LinkedTool> for LinkedToolDto {
    fn from(t: &claudepot_core::session_tool_link::LinkedTool) -> Self {
        Self {
            tool_use_id: t.tool_use_id.clone(),
            tool_name: t.tool_name.clone(),
            model: t.model.clone(),
            call_ts: t.call_ts,
            input_preview: t.input_preview.clone(),
            result_ts: t.result_ts,
            result_content: t.result_content.clone(),
            is_error: t.is_error,
            duration_ms: t.duration_ms,
            call_index: t.call_index,
            result_index: t.result_index,
        }
    }
}

#[derive(Serialize)]
pub struct ChunkHeaderDto {
    pub id: usize,
    pub start_ts: Option<DateTime<Utc>>,
    pub end_ts: Option<DateTime<Utc>>,
    pub metrics: ChunkMetricsDto,
}

impl From<&claudepot_core::session_chunks::ChunkHeader> for ChunkHeaderDto {
    fn from(h: &claudepot_core::session_chunks::ChunkHeader) -> Self {
        Self {
            id: h.id,
            start_ts: h.start_ts,
            end_ts: h.end_ts,
            metrics: (&h.metrics).into(),
        }
    }
}

/// Matches the shape the JS side already consumes: `chunkType` tag
/// flattened onto the header fields. Each variant carries the data it
/// needs to render in the transcript pane.
#[derive(Serialize)]
#[serde(tag = "chunkType", rename_all = "camelCase")]
pub enum SessionChunkDto {
    #[serde(rename = "user")]
    User {
        #[serde(flatten)]
        header: ChunkHeaderDto,
        event_index: usize,
    },
    #[serde(rename = "ai")]
    Ai {
        #[serde(flatten)]
        header: ChunkHeaderDto,
        event_indices: Vec<usize>,
        tool_executions: Vec<LinkedToolDto>,
    },
    #[serde(rename = "system")]
    System {
        #[serde(flatten)]
        header: ChunkHeaderDto,
        event_index: usize,
    },
    #[serde(rename = "compact")]
    Compact {
        #[serde(flatten)]
        header: ChunkHeaderDto,
        event_index: usize,
    },
}

impl From<&claudepot_core::session_chunks::SessionChunk> for SessionChunkDto {
    fn from(c: &claudepot_core::session_chunks::SessionChunk) -> Self {
        use claudepot_core::session_chunks::SessionChunk;
        match c {
            SessionChunk::User {
                header,
                event_index,
            } => SessionChunkDto::User {
                header: header.into(),
                event_index: *event_index,
            },
            SessionChunk::Ai {
                header,
                event_indices,
                tool_executions,
            } => SessionChunkDto::Ai {
                header: header.into(),
                event_indices: event_indices.clone(),
                tool_executions: tool_executions.iter().map(LinkedToolDto::from).collect(),
            },
            SessionChunk::System {
                header,
                event_index,
            } => SessionChunkDto::System {
                header: header.into(),
                event_index: *event_index,
            },
            SessionChunk::Compact {
                header,
                event_index,
            } => SessionChunkDto::Compact {
                header: header.into(),
                event_index: *event_index,
            },
        }
    }
}

#[derive(Serialize)]
pub struct ContextPhaseDto {
    pub phase_number: usize,
    pub start_index: usize,
    pub end_index: usize,
    pub start_ts: Option<DateTime<Utc>>,
    pub end_ts: Option<DateTime<Utc>>,
    pub summary: Option<String>,
}

impl From<&claudepot_core::session_phases::ContextPhase> for ContextPhaseDto {
    fn from(p: &claudepot_core::session_phases::ContextPhase) -> Self {
        Self {
            phase_number: p.phase_number,
            start_index: p.start_index,
            end_index: p.end_index,
            start_ts: p.start_ts,
            end_ts: p.end_ts,
            summary: p.summary.clone(),
        }
    }
}

#[derive(Serialize)]
pub struct TokensByCategoryDto {
    pub claude_md: u64,
    pub mentioned_file: u64,
    pub tool_output: u64,
    pub thinking_text: u64,
    pub team_coordination: u64,
    pub user_message: u64,
}

impl From<&claudepot_core::session_context::TokensByCategory> for TokensByCategoryDto {
    fn from(t: &claudepot_core::session_context::TokensByCategory) -> Self {
        Self {
            claude_md: t.claude_md,
            mentioned_file: t.mentioned_file,
            tool_output: t.tool_output,
            thinking_text: t.thinking_text,
            team_coordination: t.team_coordination,
            user_message: t.user_message,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContextCategoryDto {
    ClaudeMd,
    MentionedFile,
    ToolOutput,
    ThinkingText,
    TeamCoordination,
    UserMessage,
}

impl From<claudepot_core::session_context::ContextCategory> for ContextCategoryDto {
    fn from(c: claudepot_core::session_context::ContextCategory) -> Self {
        use claudepot_core::session_context::ContextCategory;
        match c {
            ContextCategory::ClaudeMd => ContextCategoryDto::ClaudeMd,
            ContextCategory::MentionedFile => ContextCategoryDto::MentionedFile,
            ContextCategory::ToolOutput => ContextCategoryDto::ToolOutput,
            ContextCategory::ThinkingText => ContextCategoryDto::ThinkingText,
            ContextCategory::TeamCoordination => ContextCategoryDto::TeamCoordination,
            ContextCategory::UserMessage => ContextCategoryDto::UserMessage,
        }
    }
}

#[derive(Serialize)]
pub struct ContextInjectionDto {
    pub event_index: usize,
    pub category: ContextCategoryDto,
    pub label: String,
    pub tokens: u64,
    pub ts: Option<DateTime<Utc>>,
    pub phase: usize,
}

impl From<&claudepot_core::session_context::ContextInjection> for ContextInjectionDto {
    fn from(i: &claudepot_core::session_context::ContextInjection) -> Self {
        Self {
            event_index: i.event_index,
            category: i.category.into(),
            label: i.label.clone(),
            tokens: i.tokens,
            ts: i.ts,
            phase: i.phase,
        }
    }
}

#[derive(Serialize)]
pub struct ContextStatsDto {
    pub totals: TokensByCategoryDto,
    pub injections: Vec<ContextInjectionDto>,
    pub phases: Vec<ContextPhaseDto>,
    pub reported_total_tokens: u64,
}

impl From<&claudepot_core::session_context::ContextStats> for ContextStatsDto {
    fn from(s: &claudepot_core::session_context::ContextStats) -> Self {
        Self {
            totals: (&s.totals).into(),
            injections: s.injections.iter().map(ContextInjectionDto::from).collect(),
            phases: s.phases.iter().map(ContextPhaseDto::from).collect(),
            reported_total_tokens: s.reported_total_tokens,
        }
    }
}

#[derive(Serialize)]
pub struct SearchHitDto {
    pub session_id: String,
    pub slug: String,
    pub file_path: String,
    pub project_path: String,
    pub role: String,
    pub snippet: String,
    pub match_offset: usize,
    pub last_ts: Option<DateTime<Utc>>,
}

impl From<&claudepot_core::session_search::SearchHit> for SearchHitDto {
    fn from(h: &claudepot_core::session_search::SearchHit) -> Self {
        Self {
            session_id: h.session_id.clone(),
            slug: h.slug.clone(),
            file_path: h.file_path.display().to_string(),
            project_path: h.project_path.clone(),
            role: h.role.clone(),
            snippet: h.snippet.clone(),
            match_offset: h.match_offset,
            last_ts: h.last_ts,
        }
    }
}

#[derive(Serialize)]
pub struct RepositoryGroupDto {
    pub repo_root: Option<String>,
    pub label: String,
    pub sessions: Vec<SessionRowDto>,
    pub branches: Vec<String>,
    pub worktree_paths: Vec<String>,
}

impl From<&claudepot_core::session_worktree::RepositoryGroup> for RepositoryGroupDto {
    fn from(g: &claudepot_core::session_worktree::RepositoryGroup) -> Self {
        Self {
            repo_root: g.repo_root.as_ref().map(|p| p.display().to_string()),
            label: g.label.clone(),
            sessions: g.sessions.iter().map(SessionRowDto::from).collect(),
            branches: g.branches.clone(),
            worktree_paths: g
                .worktree_paths
                .iter()
                .map(|p| p.display().to_string())
                .collect(),
        }
    }
}

// ---------------------------------------------------------------------------
// Keys — ANTHROPIC_API_KEY and CLAUDE_CODE_OAUTH_TOKEN management DTOs.
//
// The token itself NEVER crosses the IPC bridge on list / probe / add.
// Only the redacted preview and metadata surface. `key_*_copy` is the
// single deliberate exit — the user is explicitly asking to paste the
// token into another tool, so we return the real value there.
// ---------------------------------------------------------------------------

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

// ─── session_live DTOs ─────────────────────────────────────────────

/// Front-end view of one live Claude Code session. Mirrors
/// `claudepot_core::session_live::types::LiveSessionSummary` but with
/// chrono types pre-serialized to milliseconds-since-epoch so the
/// webview doesn't need to handle `DateTime<Utc>`.
#[derive(Serialize, Clone)]
pub struct LiveSessionSummaryDto {
    pub session_id: String,
    pub pid: u32,
    pub cwd: String,
    pub transcript_path: Option<String>,
    /// One of `busy | idle | waiting` — the canonical CC vocabulary.
    pub status: String,
    pub current_action: Option<String>,
    pub model: Option<String>,
    /// Only populated when `status == "waiting"`.
    pub waiting_for: Option<String>,
    pub errored: bool,
    pub stuck: bool,
    pub idle_ms: i64,
    pub seq: u64,
}

impl From<claudepot_core::session_live::types::LiveSessionSummary>
    for LiveSessionSummaryDto
{
    fn from(s: claudepot_core::session_live::types::LiveSessionSummary) -> Self {
        use claudepot_core::session_live::types::Status;
        let status = match s.status {
            Status::Busy => "busy",
            Status::Idle => "idle",
            Status::Waiting => "waiting",
        }
        .to_string();
        Self {
            session_id: s.session_id,
            pid: s.pid,
            cwd: s.cwd,
            transcript_path: s.transcript_path,
            status,
            current_action: s.current_action,
            model: s.model,
            waiting_for: s.waiting_for,
            errored: s.errored,
            stuck: s.stuck,
            idle_ms: s.idle_ms,
            seq: s.seq,
        }
    }
}

/// Per-session delta carried over the `live::<sessionId>` event
/// channel. The webview discriminates on `kind`; each variant
/// carries only the fields relevant to that transition.
#[derive(Serialize, Clone)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LiveDeltaKindDto {
    StatusChanged {
        status: String,
        waiting_for: Option<String>,
    },
    TaskSummaryChanged {
        summary: String,
    },
    ModelChanged {
        model: String,
    },
    OverlayChanged {
        errored: bool,
        stuck: bool,
    },
    Ended,
}

#[derive(Serialize, Clone)]
pub struct LiveDeltaDto {
    pub session_id: String,
    pub seq: u64,
    pub produced_at_ms: i64,
    #[serde(flatten)]
    pub kind: LiveDeltaKindDto,
    pub resync_required: bool,
}

impl From<claudepot_core::session_live::types::LiveDelta> for LiveDeltaDto {
    fn from(d: claudepot_core::session_live::types::LiveDelta) -> Self {
        use claudepot_core::session_live::types::{LiveDeltaKind, Status};
        let status_str = |s: Status| -> String {
            match s {
                Status::Busy => "busy",
                Status::Idle => "idle",
                Status::Waiting => "waiting",
            }
            .to_string()
        };
        let kind = match d.kind {
            LiveDeltaKind::StatusChanged {
                status,
                waiting_for,
            } => LiveDeltaKindDto::StatusChanged {
                status: status_str(status),
                waiting_for,
            },
            LiveDeltaKind::TaskSummaryChanged { summary } => {
                LiveDeltaKindDto::TaskSummaryChanged { summary }
            }
            LiveDeltaKind::ModelChanged { model } => {
                LiveDeltaKindDto::ModelChanged { model }
            }
            LiveDeltaKind::OverlayChanged { errored, stuck } => {
                LiveDeltaKindDto::OverlayChanged { errored, stuck }
            }
            LiveDeltaKind::Ended => LiveDeltaKindDto::Ended,
        };
        Self {
            session_id: d.session_id,
            seq: d.seq,
            produced_at_ms: d.produced_at_ms,
            kind,
            resync_required: d.resync_required,
        }
    }
}
