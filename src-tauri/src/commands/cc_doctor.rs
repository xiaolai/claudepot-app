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
///
/// ### Version-mismatch invalidation
///
/// Before honoring the cache TTL, this runs the cheap
/// [`claudepot_core::cc_doctor::probe_version`] probe (~50 ms,
/// non-pty) and compares its version to the cached snapshot's
/// `cc_version`. If they differ — the most likely cause is a
/// CC self-update between captures — the cache is discarded and a
/// fresh scrape runs. Without this gate, the renderer can show a
/// "claude version unknown" + "PATH not in env" snapshot for up to
/// 60 s after CC has already updated itself and resolved both
/// problems.
#[tauri::command]
pub async fn cc_doctor_snapshot(
    force_refresh: Option<bool>,
    state: tauri::State<'_, CcDoctorState>,
    tray_health: tauri::State<'_, TrayHealthState>,
    app: tauri::AppHandle,
) -> Result<DoctorSnapshotDto, String> {
    let force = force_refresh.unwrap_or(false);

    if !force {
        // Read the cached snapshot first WITHOUT holding the lock
        // across the probe — the probe is a fork-exec that takes
        // ~50 ms; we don't want to gate every other consumer on it.
        let cached: Option<(std::time::Instant, DoctorSnapshot)> = {
            let g = state
                .cache
                .lock()
                .map_err(|_| "cc_doctor cache mutex poisoned".to_string())?;
            g.as_ref().map(|c| (c.captured_at, c.snapshot.clone()))
        };

        if let Some((captured_at, snapshot)) = cached {
            if captured_at.elapsed() < CACHE_TTL {
                // TTL says fresh; check version drift before
                // returning. Probe runs on a blocking thread so a
                // hung subprocess can't pin the IPC worker — but
                // we cap at the probe's internal timeout (3s).
                let probe = tokio::task::spawn_blocking(
                    claudepot_core::cc_doctor::probe_version,
                )
                .await
                .ok()
                .flatten();
                let version_drifted = cache_should_invalidate(
                    snapshot.cc_version.as_deref(),
                    probe.as_ref().map(|p| p.version.as_str()),
                );
                if !version_drifted {
                    return Ok(snapshot.into());
                }
                tracing::info!(
                    "cc_doctor: version drift detected (cached={:?}, live={:?}) — invalidating cache",
                    snapshot.cc_version,
                    probe.as_ref().map(|p| &p.version),
                );
            }
        }
    }

    // Drop the lock across the blocking scrape — don't hold the
    // mutex through an O(seconds) operation. Multiple concurrent
    // callers will each run their own scrape (rare in practice;
    // the pill is the only caller in Cut 1), and the last write
    // wins. Worth the simplicity over a singleflight gate at this
    // size.
    //
    // Use `scrape_with_probes` instead of bare `scrape_doctor` so
    // the snapshot has the probe overlay applied — when the TUI
    // parser fails, the identity fields still come back populated
    // (see `cc_doctor::compose` for the merge rules).
    let snapshot = tokio::task::spawn_blocking(claudepot_core::cc_doctor::scrape_with_probes)
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
        .filter(|s| {
            !matches!(
                s.severity,
                DoctorSeverity::Healthy | DoctorSeverity::Unknown
            )
        })
        .count() as u32;
    let kind = match snapshot.severity {
        // Both the scrape's "we couldn't measure" verdict and the
        // tray state's pre-scrape default map to the same Unknown
        // cell — same surface (grey "checking…" copy in the tray).
        DoctorSeverity::Unknown => HealthRecordKind::Unknown,
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

/// Pure predicate that decides whether a fresh-by-TTL cache entry
/// should be discarded because the running CC version has drifted.
///
/// Inputs:
/// - `cached`: the version string we recorded when the cache was
///   filled (may be `None` if the cached scrape couldn't parse a
///   version at the time)
/// - `probe`: the version we just read live from `claude --version`
///   (may be `None` if the probe couldn't locate a binary or the
///   subprocess timed out)
///
/// Returns `true` to invalidate. Decision table:
///
/// | cached  | probe   | drift? | rationale                          |
/// |---------|---------|--------|------------------------------------|
/// | Some(a) | Some(b) | a != b | versions disagree — rescrape       |
/// | Some(a) | Some(a) | false  | equal — cache is correct           |
/// | Some(_) | None    | false  | probe failed; can't detect drift   |
/// | None    | Some(_) | true   | cache predates a known install     |
/// | None    | None    | false  | nothing to compare                 |
///
/// The `(None, None)` and `(Some, None)` cases honor the cache so a
/// flaky probe doesn't trigger a 6–10s rescrape every TTL window.
fn cache_should_invalidate(cached: Option<&str>, probe: Option<&str>) -> bool {
    match (cached, probe) {
        (Some(c), Some(p)) => c != p,
        (None, Some(_)) => true,
        (_, None) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalidate_when_versions_differ() {
        assert!(cache_should_invalidate(Some("2.1.128"), Some("2.1.140")));
    }

    #[test]
    fn keep_when_versions_match() {
        assert!(!cache_should_invalidate(Some("2.1.140"), Some("2.1.140")));
    }

    #[test]
    fn keep_when_probe_failed_with_cached_version() {
        // Probe down (subprocess hang, binary removed, etc.) — we
        // can't tell whether drift happened, so honor the cache.
        // Forcing a rescrape on every probe failure would burn the
        // 6–10s pty cost during transient subprocess flakiness.
        assert!(!cache_should_invalidate(Some("2.1.140"), None));
    }

    #[test]
    fn invalidate_when_cache_has_no_version_but_probe_does() {
        // The previously cached snapshot couldn't parse a version
        // (scrape failed without the probe overlay populating
        // cc_version, or pre-overlay history). A fresh probe with
        // a version is strictly more information — rescrape.
        assert!(cache_should_invalidate(None, Some("2.1.140")));
    }

    #[test]
    fn keep_when_both_unknown() {
        // Nothing to compare; the cache and probe agree on "no
        // signal". Don't spin a rescrape that would also yield no
        // signal.
        assert!(!cache_should_invalidate(None, None));
    }
}
