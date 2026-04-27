//! Tauri commands for artifact-usage telemetry.
//!
//! Read-only surface backed by `claudepot_core::artifact_usage` over
//! `sessions.db`. Every handler:
//!
//! - opens the session index,
//! - calls `refresh()` so the data is current (the session-index
//!   refresh is idempotent and cheap when nothing changed),
//! - then queries via the public `SessionIndex::usage_*` API.
//!
//! All handlers run in `spawn_blocking` because the refresh path
//! parses JSONL and SQLite is sync.
//!
//! The handlers do not contain business logic — they are thin
//! adapters over the core API per `architecture.md`.

use crate::dto_artifact_usage::{
    parse_kind, ArtifactUsageBatchEntryDto, ArtifactUsageRowDto, ArtifactUsageStatsDto,
};
use chrono::Utc;
use claudepot_core::paths;
use claudepot_core::session_index::SessionIndex;

fn join_blocking_err(e: tokio::task::JoinError) -> String {
    format!("blocking task failed: {e}")
}

/// Open the index at `<data>/sessions.db` and run a refresh against
/// `<config>/projects/`. Centralized here so every usage command
/// applies the same freshness contract.
fn open_and_refresh() -> Result<SessionIndex, String> {
    let data_dir = paths::claudepot_data_dir();
    let db_path = data_dir.join("sessions.db");
    let idx = SessionIndex::open(&db_path).map_err(|e| format!("open session index: {e}"))?;
    let cfg = paths::claude_config_dir();
    idx.refresh(&cfg)
        .map_err(|e| format!("refresh session index: {e}"))?;
    Ok(idx)
}

/// One artifact's stats. Empty stats are returned (not an error) for
/// artifacts that have never fired — the UI uses `count_30d == 0`
/// to render the "never used" state.
#[tauri::command]
pub async fn artifact_usage_for(
    kind: String,
    artifact_key: String,
) -> Result<ArtifactUsageStatsDto, String> {
    tokio::task::spawn_blocking(move || {
        let kind = parse_kind(&kind)?;
        let idx = open_and_refresh()?;
        let now_ms = Utc::now().timestamp_millis();
        let stats = idx
            .usage_for_artifact(kind, &artifact_key, now_ms)
            .map_err(|e| format!("query: {e}"))?;
        Ok::<_, String>(stats.into())
    })
    .await
    .map_err(join_blocking_err)?
}

/// Batch fetch — used by the Config-tree renderer to populate badges
/// for every visible artifact in one round-trip.
///
/// Returns one entry per resolvable `(kind, key)` in input order.
/// Invalid kinds are silently skipped (UI shouldn't have produced
/// them; this keeps a malformed renderer call from killing the whole
/// batch).
#[tauri::command]
pub async fn artifact_usage_batch(
    keys: Vec<(String, String)>,
) -> Result<Vec<ArtifactUsageBatchEntryDto>, String> {
    tokio::task::spawn_blocking(move || {
        // Resolve kinds up-front so the core batch sees only valid pairs.
        let parsed: Vec<(claudepot_core::artifact_usage::ArtifactKind, String)> = keys
            .into_iter()
            .filter_map(|(k, v)| parse_kind(&k).ok().map(|kind| (kind, v)))
            .collect();
        if parsed.is_empty() {
            return Ok::<_, String>(Vec::new());
        }
        let idx = open_and_refresh()?;
        let now_ms = Utc::now().timestamp_millis();
        let rows = idx
            .usage_batch(&parsed, now_ms)
            .map_err(|e| format!("query: {e}"))?;
        Ok::<_, String>(
            rows.into_iter()
                .map(|((kind, key), stats)| ArtifactUsageBatchEntryDto {
                    kind: kind.as_str().to_string(),
                    artifact_key: key,
                    stats: stats.into(),
                })
                .collect(),
        )
    })
    .await
    .map_err(join_blocking_err)?
}

/// Top N artifacts by 30-day fire count. Optional kind filter.
#[tauri::command]
pub async fn artifact_usage_top(
    kind: Option<String>,
    limit: u32,
) -> Result<Vec<ArtifactUsageRowDto>, String> {
    tokio::task::spawn_blocking(move || {
        let kind = match kind.as_deref() {
            Some(s) => Some(parse_kind(s)?),
            None => None,
        };
        let idx = open_and_refresh()?;
        let now_ms = Utc::now().timestamp_millis();
        let rows = idx
            .usage_top(kind, limit as usize, now_ms)
            .map_err(|e| format!("query: {e}"))?;
        Ok::<_, String>(rows.into_iter().map(ArtifactUsageRowDto::from).collect())
    })
    .await
    .map_err(join_blocking_err)?
}

// `artifact_usage_known_keys` was a stub for an "Unused" filter that
// never shipped in this slice. Keep `SessionIndex::usage_known_keys`
// (cheap, no Tauri surface) for the inevitable Slice 5; this Tauri
// command, the JS API, and the unused TS wiring are all removed to
// keep the IPC surface honest about what the UI actually consumes.
