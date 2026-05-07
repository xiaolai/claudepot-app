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
use claudepot_core::cc_tips::catalog::{ensure_catalog, record_view, render_tips};
use claudepot_core::cc_tips::triggers::known_id_count;

#[tauri::command]
pub async fn cc_tips_list() -> Result<TipsRenderDto, String> {
    render_tips(false)
        .map(TipsRenderDto::from)
        .map_err(|e| format!("cc_tips_list: {e}"))
}

#[tauri::command]
pub async fn cc_tips_refresh() -> Result<TipsRefreshDto, String> {
    let snap = ensure_catalog(true).map_err(|e| format!("cc_tips_refresh: {e}"))?;
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
pub async fn cc_tips_record_view() -> Result<bool, String> {
    record_view().map_err(|e| format!("cc_tips_record_view: {e}"))
}
