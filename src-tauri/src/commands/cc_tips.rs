//! Tauri surface for the CC tips ledger. Three commands; pure
//! pass-through over `claudepot_core::cc_tips::catalog`.
//!
//! - `cc_tips_list` — render the current tips view (cached catalog
//!   + tipsHistory join + snapshot-resolved last-seen).
//! - `cc_tips_refresh` — force re-extraction from the user's CC
//!   binary, overwrite the cache, and return the new totals.
//! - `cc_tips_record_view` — append a snapshot if more than 1 hour
//!   has passed since the last (called from the UI on Tips-view
//!   mount). Used for snapshot-diff time resolution.

use crate::dto_cc_tips::{TipsRefreshDto, TipsRenderDto};
use crate::dto_error::ErrorDto;
use claudepot_core::cc_tips::catalog::{ensure_catalog, record_view, render_tips};
use claudepot_core::cc_tips::triggers::known_id_count;

/// On a cache miss `render_tips` reads the whole CC binary
/// (~150 MB) synchronously, so it runs on a blocking thread.
///
/// The per-command prefixes (`cc_tips_list: …`) are gone — `TipsError`
/// already names what failed, and the UI owns the framing for what it
/// was attempting. See `crate::dto_error`.
#[tauri::command]
pub async fn cc_tips_list() -> Result<TipsRenderDto, ErrorDto> {
    tokio::task::spawn_blocking(|| render_tips(false))
        .await
        .map_err(ErrorDto::task_join)?
        .map(TipsRenderDto::from)
        .map_err(ErrorDto::from)
}

/// Forced refresh always re-reads the whole CC binary, so it runs
/// on a blocking thread.
#[tauri::command]
pub async fn cc_tips_refresh() -> Result<TipsRefreshDto, ErrorDto> {
    let snap = tokio::task::spawn_blocking(|| ensure_catalog(true))
        .await
        .map_err(ErrorDto::task_join)??;
    let extracted = snap.tips.len();
    let known = known_id_count();
    let partial = extracted * 100 / known.max(1) < 80;
    Ok(TipsRefreshDto {
        extracted_count: extracted,
        known_count: known,
        partial,
        catalog_version: snap.cc_version,
    })
}

#[tauri::command]
pub async fn cc_tips_record_view() -> Result<bool, ErrorDto> {
    record_view().map_err(ErrorDto::from)
}
