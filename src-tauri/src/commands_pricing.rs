//! Pricing commands — expose `claudepot_core::pricing` to the
//! frontend.
//!
//! The frontend calls `pricing_get` on mount and whenever it wants a
//! fresh snapshot for display. `pricing_get` is cheap: it returns the
//! current best in-memory snapshot via the
//! [`PricingCacheService`] and never blocks on the network. If the
//! current source is `Bundled`, the service kicks a background
//! refresh so the next call returns fresh numbers — singleflighted
//! across all callers (D-3).
//!
//! `pricing_refresh` forces a refresh and joins the in-flight task if
//! one exists, so a button-mash on "Refresh rates" can no longer
//! spawn N concurrent scrapes.

use claudepot_core::pricing::{ModelRates, PriceSource, PriceTable, PricingCacheService};
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::UNIX_EPOCH;
use tauri::State;

#[derive(Serialize, Clone, Debug)]
pub struct PriceSourceDto {
    /// "bundled" | "live" | "cached"
    pub kind: String,
    /// ISO-8601 string when the number is meaningful (live / cached);
    /// otherwise the date the bundled rates were last verified.
    pub timestamp: String,
    /// URL for the scrape target (live / cached) or empty for bundled.
    pub url: String,
}

#[derive(Serialize, Clone, Debug)]
pub struct ModelRatesDto {
    pub input_per_mtok: f64,
    pub output_per_mtok: f64,
    pub cache_write_per_mtok: f64,
    pub cache_read_per_mtok: f64,
}

#[derive(Serialize, Clone, Debug)]
pub struct PriceTableDto {
    pub models: BTreeMap<String, ModelRatesDto>,
    pub source: PriceSourceDto,
    /// Short user-safe message when the last refresh attempt failed.
    /// `None` = last refresh succeeded or was never attempted.
    pub last_fetch_error: Option<String>,
}

impl From<ModelRates> for ModelRatesDto {
    fn from(r: ModelRates) -> Self {
        ModelRatesDto {
            input_per_mtok: r.input_per_mtok,
            output_per_mtok: r.output_per_mtok,
            cache_write_per_mtok: r.cache_write_per_mtok,
            cache_read_per_mtok: r.cache_read_per_mtok,
        }
    }
}

fn unix_to_iso(unix: u64) -> String {
    // Lightweight ISO-ish formatter without pulling in chrono. Good
    // enough for a tooltip — `YYYY-MM-DD HH:MM:SSZ`. If the system
    // clock is before 1970 (doesn't happen in practice) we return a
    // bland placeholder.
    use std::time::Duration;
    let d = Duration::from_secs(unix);
    let t = UNIX_EPOCH.checked_add(d);
    let Some(_) = t else { return "—".to_string() };
    let secs = unix as i64;
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let h = (rem / 3600) as u32;
    let m = ((rem % 3600) / 60) as u32;
    let s = (rem % 60) as u32;
    // Days since 1970-01-01 → Gregorian date. Small lookup for the
    // current ~60-year range; exact to the day.
    let (y, mo, da) = gregorian_from_days(days);
    format!("{y:04}-{mo:02}-{da:02} {h:02}:{m:02}:{s:02}Z")
}

fn gregorian_from_days(days: i64) -> (i32, u32, u32) {
    // Algorithm adapted from the civil_from_days reference
    // implementation (Howard Hinnant's public-domain date math).
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let y = if m <= 2 { y + 1 } else { y } as i32;
    (y, m, d)
}

fn to_source_dto(src: PriceSource) -> PriceSourceDto {
    match src {
        PriceSource::Bundled { verified_at } => PriceSourceDto {
            kind: "bundled".to_string(),
            timestamp: verified_at,
            url: String::new(),
        },
        PriceSource::Live { url, fetched_at_unix } => PriceSourceDto {
            kind: "live".to_string(),
            timestamp: unix_to_iso(fetched_at_unix),
            url,
        },
        PriceSource::Cached {
            fetched_at_unix,
            source_url,
        } => PriceSourceDto {
            kind: "cached".to_string(),
            timestamp: unix_to_iso(fetched_at_unix),
            url: source_url,
        },
    }
}

fn to_table_dto(table: &PriceTable) -> PriceTableDto {
    // Clone into the DTO. Reading from an `Arc<PriceTable>` snapshot
    // means the source data is immutable; we only need the clone so
    // the returned DTO can own its own `String`s and rate copies for
    // serde-serialization across the IPC bridge.
    let models = table
        .models
        .iter()
        .map(|(k, v)| (k.clone(), v.clone().into()))
        .collect();
    PriceTableDto {
        models,
        source: to_source_dto(table.source.clone()),
        last_fetch_error: table.last_fetch_error.clone(),
    }
}

/// Return the best currently-available price table. Never blocks on
/// the network: when the in-memory snapshot is `Bundled`, the
/// underlying service spawns a background refresh (singleflighted
/// across all callers) so the *next* call returns fresh numbers
/// without making this one wait.
#[tauri::command]
pub async fn pricing_get(
    svc: State<'_, Arc<PricingCacheService>>,
) -> Result<PriceTableDto, String> {
    let table = svc.get_or_refresh_async();
    Ok(to_table_dto(table.as_ref()))
}

/// Force a refresh right now and return the fresh table. If a refresh
/// is already in flight (kicked by `pricing_get` or a previous button
/// press), joins it instead of spawning a duplicate. On fetch failure
/// the service returns a bundled fallback tagged with
/// `last_fetch_error` — never panics, never poisons the singleflight.
#[tauri::command]
pub async fn pricing_refresh(
    svc: State<'_, Arc<PricingCacheService>>,
) -> Result<PriceTableDto, String> {
    let table = svc.refresh_now().await;
    Ok(to_table_dto(table.as_ref()))
}
