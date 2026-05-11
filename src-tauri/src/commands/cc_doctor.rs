//! Tauri commands for the `claude doctor` scrape pipeline.
//!
//! One command for now: [`cc_doctor_snapshot`]. The renderer polls it
//! every 60s for the WindowChrome health pill. Caching is process-
//! local — a 60s wall-clock cache keeps the cost low when multiple
//! UI consumers ask within the same minute (the pane added in
//! Cut 2 will share this cache with the pill).
//!
//! Per `.claude/rules/architecture.md`: no business logic here.
//! Wraps [`claudepot_core::cc_doctor::scrape_doctor`], converts the
//! core type to a DTO, and caches.

use std::sync::Mutex;
use std::time::{Duration, Instant};

use claudepot_core::cc_doctor::{DoctorSeverity, DoctorSnapshot};

use crate::dto_cc_doctor::DoctorSnapshotDto;
use crate::state::{HealthRecordKind, TrayHealthState};

/// How long a snapshot stays "fresh" before the next call re-scrapes.
/// 60s matches the renderer's 60s poll cadence — same-second double
/// callers (pill + pane in Cut 2) hit the cache. The scrape itself
/// takes ~6–10 s including the npm dist-tag fetch, so without the
/// cache the second caller would pay that cost in full.
const CACHE_TTL: Duration = Duration::from_secs(60);

#[derive(Debug, Clone)]
struct Cached {
    /// Wall-clock when the scrape STARTED (not when it returned).
    /// Aligns with `captured_at_ms` in the snapshot.
    captured_at: Instant,
    snapshot: DoctorSnapshot,
}

/// Tauri-managed handle. Cheap to construct (the inner option is
/// `None` until the first scrape runs).
pub struct CcDoctorState {
    cache: Mutex<Option<Cached>>,
}

impl Default for CcDoctorState {
    fn default() -> Self {
        Self {
            cache: Mutex::new(None),
        }
    }
}

impl CcDoctorState {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Read the cached snapshot or scrape afresh.
///
/// `force_refresh = Some(true)` bypasses the cache (used by a future
/// "Refresh" button in the pane and by the dev-mode test harness).
/// `None` or `Some(false)` honors the 60 s TTL.
///
/// The pty scrape is blocking — we run it on a tokio blocking thread
/// so the Tauri command worker isn't tied up for the 6–10 s scrape
/// window.
#[tauri::command]
pub async fn cc_doctor_snapshot(
    force_refresh: Option<bool>,
    state: tauri::State<'_, CcDoctorState>,
    tray_health: tauri::State<'_, TrayHealthState>,
    app: tauri::AppHandle,
) -> Result<DoctorSnapshotDto, String> {
    let force = force_refresh.unwrap_or(false);

    if !force {
        let g = state
            .cache
            .lock()
            .map_err(|_| "cc_doctor cache mutex poisoned".to_string())?;
        if let Some(c) = g.as_ref() {
            if c.captured_at.elapsed() < CACHE_TTL {
                return Ok(c.snapshot.clone().into());
            }
        }
    }

    // Drop the lock across the blocking scrape — don't hold the
    // mutex through an O(seconds) operation. Multiple concurrent
    // callers will each run their own scrape (rare in practice;
    // the pill is the only caller in Cut 1), and the last write
    // wins. Worth the simplicity over a singleflight gate at this
    // size.
    let snapshot = tokio::task::spawn_blocking(claudepot_core::cc_doctor::scrape_doctor)
        .await
        .map_err(|e| format!("cc_doctor blocking-task join: {e}"))?;

    {
        let mut g = state
            .cache
            .lock()
            .map_err(|_| "cc_doctor cache mutex poisoned".to_string())?;
        *g = Some(Cached {
            captured_at: Instant::now(),
            snapshot: snapshot.clone(),
        });
    }

    // Tray-side mirror: every fresh scrape updates TrayHealthState
    // and asks for a tray rebuild so the menu label stays in sync.
    // Rebuild is async (peeks usage cache); spawn it without
    // awaiting — the snapshot return path must not block on the
    // tray refresh.
    push_to_tray_health(&tray_health, &snapshot);
    let app_for_rebuild = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = crate::tray::rebuild(&app_for_rebuild).await {
            tracing::warn!("cc_doctor: tray rebuild after scrape failed: {e}");
        }
    });

    Ok(snapshot.into())
}

/// Map a [`DoctorSnapshot`] into the tray's coarser state shape and
/// write it to [`TrayHealthState`]. Pure function; exposed so the
/// background poller (`cc_doctor_watcher`) can reuse the same
/// mapping rather than diverge.
pub fn push_to_tray_health(state: &TrayHealthState, snapshot: &DoctorSnapshot) {
    let flagged = snapshot
        .sections
        .iter()
        .filter(|s| !matches!(s.severity, DoctorSeverity::Healthy))
        .count() as u32;
    let kind = match snapshot.severity {
        DoctorSeverity::Healthy => HealthRecordKind::Healthy,
        DoctorSeverity::Warning => HealthRecordKind::Warning,
        DoctorSeverity::Error => HealthRecordKind::Error,
    };
    state.set(kind, flagged);
}

/// Reveal the parse-failures forensic log in the OS file manager,
/// or — when the log doesn't exist yet (no parse failure has been
/// recorded on this machine) — reveal its parent data directory so
/// the user lands somewhere navigable instead of getting an error.
///
/// Wraps [`crate::commands::reveal_in_finder`] so the underlying
/// macOS/Linux/Windows branching stays in one place.
#[tauri::command]
pub async fn cc_doctor_open_parse_failures_log() -> Result<(), String> {
    let path = claudepot_core::cc_doctor::parse_failures::default_path();
    let target = if path.exists() {
        path
    } else {
        // The file is only created on the first parse failure; on a
        // healthy machine it never exists. Reveal the parent dir so
        // the user sees the other ring-buffer files
        // (notifications.json, rotation-audit.json) and can confirm
        // there's just been nothing to log.
        claudepot_core::paths::claudepot_data_dir()
    };
    crate::commands::reveal_in_finder(target.display().to_string()).await
}
