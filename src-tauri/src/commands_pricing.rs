//! Pricing commands — expose `claudepot_core::pricing` to the
//! frontend and kick a background refresh when the cache is stale.
//!
//! The frontend calls `pricing_get` on mount and whenever it wants a
//! fresh snapshot for display. The command is cheap: it reads the
//! file cache, or returns bundled defaults, in a few ms. If the
//! cache is stale, it also spawns a non-blocking `pricing_refresh`
//! task so the next read returns fresh numbers without the caller
//! waiting on the network.

use claudepot_core::pricing::{self, ModelRates, PriceSource, PriceTable};
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::UNIX_EPOCH;

/// Singleflight guard: when `pricing_get` is called rapidly from
/// several surfaces (dashboard mount + transcript header + sidebar
/// badge all at once) and the table is bundled, we'd otherwise spawn
/// N concurrent scrapes of the same URL. This flag ensures only one
/// refresh task is in flight across the whole process.
static REFRESH_IN_FLIGHT: AtomicBool = AtomicBool::new(false);

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

fn to_table_dto(table: PriceTable) -> PriceTableDto {
    let models = table
        .models
        .into_iter()
        .map(|(k, v)| (k, v.into()))
        .collect();
    PriceTableDto {
        models,
        source: to_source_dto(table.source),
        last_fetch_error: table.last_fetch_error,
    }
}

/// Return the best currently-available price table. Never blocks on
/// the network: if the cache is stale, a background refresh is
/// spawned so the *next* call returns fresh numbers without making
/// this one wait.
#[tauri::command]
pub async fn pricing_get() -> Result<PriceTableDto, String> {
    let table = pricing::load();
    // If we fell back to bundled (or the cache is about to expire),
    // fire a fresh fetch in the background. The task doesn't own the
    // UI — its result goes to the file cache and is picked up by the
    // next `pricing_get` call.
    let needs_refresh = matches!(table.source, PriceSource::Bundled { .. });
    if needs_refresh {
        // Singleflight: only spawn when no other refresh is running.
        // `compare_exchange` returns Ok iff we were the one that
        // flipped the flag from false → true, which is the only
        // caller that should drive the spawn.
        if REFRESH_IN_FLIGHT
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            tauri::async_runtime::spawn(async {
                // Ignore errors — bundled defaults are already serving
                // the UI. `pricing::refresh_now` persists to cache on
                // success and logs its own warnings on failure.
                let _ = pricing::refresh_now().await;
                REFRESH_IN_FLIGHT.store(false, Ordering::Release);
            });
        }
    }
    Ok(to_table_dto(table))
}

/// Force a refresh right now and return the fresh table (or the
/// previous-best if the fetch fails). Useful for a "Refresh rates"
/// button; the frontend dashboard doesn't need it for the automatic
/// daily cadence — that happens transparently via `pricing_get`.
#[tauri::command]
pub async fn pricing_refresh() -> Result<PriceTableDto, String> {
    match pricing::refresh_now().await {
        Ok(fresh) => Ok(to_table_dto(fresh)),
        Err(e) => {
            // Keep serving the previous-best table, but annotate the
            // error so the UI can show "couldn't refresh: <reason>".
            let mut fallback = pricing::load();
            fallback.last_fetch_error = Some(e);
            Ok(to_table_dto(fallback))
        }
    }
}
