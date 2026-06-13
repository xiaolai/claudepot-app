//! Channel-aware self-updater commands for Claudepot's own app bundle.
//!
//! # Why these exist (the verified hard constraint)
//!
//! The JavaScript `@tauri-apps/plugin-updater` `check()` **cannot**
//! override the manifest endpoint — `CheckOptions` has no `endpoints`
//! field, and the plugin-registration `Builder` lacks one too. The
//! *only* runtime endpoint override is the Rust `UpdaterBuilder`
//! returned by `app.updater_builder()` (the `UpdaterExt` trait):
//! `endpoints(Vec<Url>)`.
//!
//! A user-selectable release channel therefore has to drive the
//! check/download/install from Rust. These commands replace the JS
//! plugin's `check()` / `downloadAndInstall()` for the channel path:
//!
//! - [`release_update_check`] reads the persisted channel, builds
//!   `app.updater_builder().endpoints(<channel endpoints>)?.build()?`,
//!   runs `.check()`, and **stashes** the resulting Rust `Update` in
//!   the [`ReleaseUpdateState`] managed `Mutex<Option<Update>>`. It
//!   returns a [`ReleaseUpdateCheckDto`].
//! - [`release_update_install`] takes the stashed `Update` and runs
//!   `download_and_install`, emitting `release-update://download`
//!   progress events. The renderer then relaunches via
//!   `tauri_plugin_process`.
//! - [`release_channel_get`] / [`release_channel_set`] read and write
//!   the `release_channel` preference. A channel switch takes effect
//!   on the *next* check — `release_update_check` reads the pref each
//!   call — so no app restart is needed. A switch also invalidates
//!   any stashed `Update`: the handle is bound to the endpoints it
//!   was checked against, and installing it after a switch would
//!   ship the *other* channel's build.
//! - [`release_relaunch_busy_ops`] is the pre-relaunch quiesce probe
//!   — the renderer calls it before `relaunch()` so a restart-to-
//!   update can warn-confirm instead of killing in-flight work.
//!
//! Per `.claude/rules/architecture.md` no business logic lives here:
//! the channel → endpoint mapping is pure logic in
//! `claudepot_core::release_channel`; this module only bridges it to
//! the Tauri updater runtime.

use claudepot_core::release_channel::ReleaseChannel;
use serde::Serialize;
use std::sync::{Arc, Mutex};
use tauri::{Emitter, Url};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::preferences::PreferencesState;

/// Event name the download-progress payloads are emitted on. The
/// renderer subscribes to this for the duration of an install.
pub const DOWNLOAD_EVENT: &str = "release-update://download";

/// Tauri-managed holder for the most recent checked `Update`.
///
/// `release_update_check` stashes the Rust `Update` here so
/// `release_update_install` can act on the *same* handle — the
/// `Update` is bound to the channel endpoints it was checked against,
/// and there is no way to reconstruct it from a DTO. A fresh check
/// overwrites the slot; `release_update_install` clears it on a
/// successful install and `release_channel_set` clears it on a
/// channel switch, so a stale handle can't be re-used.
#[derive(Default)]
pub struct ReleaseUpdateState(pub Mutex<Option<Update>>);

impl ReleaseUpdateState {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Result of a channel-aware update check, marshalled to the renderer.
///
/// `update_available == false` means the check completed and the
/// install is current — `version` / `notes` / `pub_date` are then
/// `None`. A failed check surfaces as an `Err(String)` from the
/// command, never as this struct.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReleaseUpdateCheckDto {
    /// Whether the manifest announced a newer version than the
    /// running build.
    pub update_available: bool,
    /// The announced version (no leading `v`). `None` when up to date.
    pub version: Option<String>,
    /// The currently-running version, always present.
    pub current_version: String,
    /// Release notes / changelog body from the manifest. `None` when
    /// up to date or when the manifest omitted notes.
    pub notes: Option<String>,
    /// Publish date as `YYYY-MM-DD`, if the manifest carried one.
    pub pub_date: Option<String>,
    /// The channel this check ran against — echoed back so the
    /// renderer can confirm which manifest it is looking at.
    pub channel: String,
    /// True when the check ran on the Stable channel from a running
    /// *prerelease* build and the stable manifest's newest version is
    /// older than the running version (the Beta → Stable switch
    /// case). The user is "stranded": not on the latest stable, but
    /// the stable channel has nothing newer to offer until it passes
    /// the running prerelease. The UI must not render "you're on the
    /// latest version" in this state.
    pub stranded_on_prerelease: bool,
    /// The stable manifest's current version when stranded (no
    /// leading `v`). `None` otherwise.
    pub stable_version: Option<String>,
}

/// One download-progress tick emitted on [`DOWNLOAD_EVENT`].
///
/// Mirrors the JS plugin's `DownloadEvent` shape (`Started` /
/// `Progress` / `Finished`) so the renderer's progress handling
/// stays structurally identical to the pre-rewire code.
#[derive(Debug, Clone, Serialize)]
#[serde(
    tag = "event",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum DownloadProgress {
    /// Download has begun. `content_length` is the total byte count
    /// when the server sent a `Content-Length`, else `None`.
    Started { content_length: Option<u64> },
    /// A chunk arrived. `downloaded` is the cumulative byte count.
    Progress {
        downloaded: u64,
        content_length: Option<u64>,
    },
    /// The full payload has been fetched and verified; install is
    /// about to run.
    Finished,
}

/// Read the persisted release channel from the preferences state.
fn channel_from_prefs(
    prefs: &tauri::State<'_, PreferencesState>,
) -> Result<ReleaseChannel, String> {
    Ok(prefs
        .0
        .lock()
        .map_err(|e| format!("preferences lock: {e}"))?
        .release_channel)
}

/// Resolve a [`ReleaseChannel`] to the parsed `Url` list the
/// `UpdaterBuilder` expects. The channel module returns `&str`
/// endpoints (it stays free of the `url` crate); parsing happens
/// here at the Tauri boundary.
fn channel_endpoints(channel: ReleaseChannel) -> Result<Vec<Url>, String> {
    channel
        .endpoints()
        .into_iter()
        .map(|s| Url::parse(s).map_err(|e| format!("invalid updater endpoint {s:?}: {e}")))
        .collect()
}

/// Read the current release channel preference.
#[tauri::command]
pub async fn release_channel_get(
    prefs: tauri::State<'_, PreferencesState>,
) -> Result<String, String> {
    Ok(channel_from_prefs(&prefs)?.as_str().to_string())
}

/// Persist a new release channel. Accepts `"stable"` or `"beta"`
/// (case-insensitive). The change takes effect on the *next*
/// [`release_update_check`] — no restart needed, because the check
/// command reads the preference each call.
///
/// An actual channel *change* also clears the [`ReleaseUpdateState`]
/// stash: the stashed `Update` is bound to the endpoints it was
/// checked against, so installing it after a switch would ship the
/// other channel's build.
///
/// Returns the normalized channel string so the renderer mirrors the
/// canonical value rather than whatever casing the user's `<select>`
/// emitted.
#[tauri::command]
pub async fn release_channel_set(
    prefs: tauri::State<'_, PreferencesState>,
    state: tauri::State<'_, ReleaseUpdateState>,
    channel: String,
) -> Result<String, String> {
    let parsed: ReleaseChannel = channel.parse()?;
    // Mutate the in-memory snapshot under the std::sync guard, drop
    // the guard, then persist on a blocking task — the mutex must not
    // be held across the disk write (every other preferences reader
    // contends for it). Same discipline as `preferences_set_*`.
    let (snapshot, changed) = {
        let mut p = prefs
            .0
            .lock()
            .map_err(|e| format!("preferences lock: {e}"))?;
        let changed = p.release_channel != parsed;
        p.release_channel = parsed;
        (p.clone(), changed)
    };
    if changed {
        // Invalidate before persisting: even if the disk write below
        // fails, the in-memory preference (which every check reads)
        // already carries the new channel, so the old stash is stale
        // either way.
        *state
            .0
            .lock()
            .map_err(|e| format!("update state lock: {e}"))? = None;
    }
    tokio::task::spawn_blocking(move || snapshot.save())
        .await
        .map_err(|e| format!("blocking task failed: {e}"))??;
    Ok(parsed.as_str().to_string())
}

/// Channel-aware update check.
///
/// Reads the persisted channel, builds an `Updater` whose endpoints
/// are the channel's manifest URL(s), runs `.check()`, and stashes
/// the resulting `Update` in [`ReleaseUpdateState`] for a later
/// [`release_update_install`]. Returns a [`ReleaseUpdateCheckDto`].
///
/// The `pubkey` is *not* re-set here — `app.updater_builder()` seeds
/// the builder from `tauri.conf.json` `plugins.updater`, which
/// already carries the production pubkey. Overriding only the
/// endpoints leaves signature verification anchored to the same key.
#[tauri::command]
pub async fn release_update_check(
    app: tauri::AppHandle,
    prefs: tauri::State<'_, PreferencesState>,
    state: tauri::State<'_, ReleaseUpdateState>,
) -> Result<ReleaseUpdateCheckDto, String> {
    let channel = channel_from_prefs(&prefs)?;
    let endpoints = channel_endpoints(channel)?;

    // Stranded-on-prerelease probe (the Beta → Stable switch case).
    // The plugin's default comparator is strictly-greater, so a user
    // running 0.2.0-beta.1 who checks the Stable channel (currently
    // at, say, 0.1.46) gets `None` — indistinguishable from genuinely
    // being up to date. When the running build is a prerelease and
    // the channel is Stable, override the comparator to also surface
    // a *lower* manifest version, record whether the remote was
    // actually newer, and classify below via `is_stranded`.
    let stranded_probe =
        channel == ReleaseChannel::Stable && !app.package_info().version.pre.is_empty();
    let remote_newer: Arc<Mutex<Option<bool>>> = Arc::new(Mutex::new(None));

    // `updater_builder()` seeds endpoints + pubkey from
    // tauri.conf.json; `.endpoints(...)` overrides only the endpoint
    // list, leaving the pubkey (and thus signature verification)
    // intact.
    let mut builder = app
        .updater_builder()
        .endpoints(endpoints)
        .map_err(|e| format!("updater endpoint config failed: {e}"))?;
    if stranded_probe {
        let flag = Arc::clone(&remote_newer);
        builder = builder.version_comparator(move |current, release| {
            if let Ok(mut g) = flag.lock() {
                *g = Some(release.version > current);
            }
            // Surface any *differing* release so the stranded case
            // still yields the manifest's version; an equal version
            // keeps the plain up-to-date path.
            release.version != current
        });
    }
    let updater = builder
        .build()
        .map_err(|e| format!("updater build failed: {e}"))?;

    let maybe_update = updater
        .check()
        .await
        .map_err(|e| format!("update check failed: {e}"))?;

    match maybe_update {
        None => {
            // Up to date. Clear any previously-stashed handle so a
            // stale install can't fire against an outdated check.
            let current = app.package_info().version.to_string();
            *state
                .0
                .lock()
                .map_err(|e| format!("update state lock: {e}"))? = None;
            Ok(ReleaseUpdateCheckDto {
                update_available: false,
                version: None,
                current_version: current,
                notes: None,
                pub_date: None,
                channel: channel.as_str().to_string(),
                stranded_on_prerelease: false,
                stable_version: None,
            })
        }
        Some(update) if is_stranded(stranded_probe, remote_newer.lock().ok().and_then(|g| *g)) => {
            // The stable manifest's newest version is *older* than
            // this running prerelease — the user is stranded, not up
            // to date. Don't stash the handle: installing it would
            // sidegrade to an older build the user never asked for.
            *state
                .0
                .lock()
                .map_err(|e| format!("update state lock: {e}"))? = None;
            Ok(ReleaseUpdateCheckDto {
                update_available: false,
                version: None,
                current_version: update.current_version.clone(),
                notes: None,
                pub_date: None,
                channel: channel.as_str().to_string(),
                stranded_on_prerelease: true,
                stable_version: Some(update.version.clone()),
            })
        }
        Some(update) => {
            // `time::OffsetDateTime::date()` yields a `time::Date`
            // whose Display is `YYYY-MM-DD` — exactly the form the
            // UI renders. Going through `.date()` avoids pulling in
            // `time`'s format-description machinery as a direct dep.
            let pub_date = update.date.map(|d| d.date().to_string());
            let dto = ReleaseUpdateCheckDto {
                update_available: true,
                version: Some(update.version.clone()),
                current_version: update.current_version.clone(),
                notes: update.body.clone(),
                pub_date,
                channel: channel.as_str().to_string(),
                stranded_on_prerelease: false,
                stable_version: None,
            };
            *state
                .0
                .lock()
                .map_err(|e| format!("update state lock: {e}"))? = Some(update);
            Ok(dto)
        }
    }
}

/// Classify a check that surfaced a remote release.
///
/// `stranded_probe` — the check ran on the Stable channel from a
/// running prerelease build (the only configuration where the
/// strictly-greater default comparator can mask a real difference).
/// `remote_newer` — whether the manifest version was strictly greater
/// than the running version; `None` when the probe comparator never
/// ran (probe inactive, in which case the plugin's own
/// strictly-greater comparator already vouched for "newer").
///
/// Stranded means: probe active AND the surfaced release is *not*
/// newer — i.e. the stable channel's newest version is older than the
/// running prerelease.
fn is_stranded(stranded_probe: bool, remote_newer: Option<bool>) -> bool {
    stranded_probe && !remote_newer.unwrap_or(true)
}

/// Pre-relaunch quiesce probe: labels of background ops still in
/// `Running` status. The renderer calls this before `relaunch()` so a
/// restart-to-update can warn-confirm instead of killing in-flight
/// work — a half-completed credential swap or CC auto-install is not
/// journal-protected the way repair ops are. "Busy" is defined
/// exactly as the quit gate defines it (`app_menu::attempt_quit`):
/// any [`crate::ops::RunningOps`] entry still `Running`. Zero
/// overhead in the common idle case — a single map scan.
#[tauri::command]
pub async fn release_relaunch_busy_ops(
    ops: tauri::State<'_, crate::ops::RunningOps>,
) -> Result<Vec<String>, String> {
    Ok(ops
        .list()
        .into_iter()
        .filter(|op| op.status == crate::ops::OpStatus::Running)
        .map(|op| crate::app_menu::inflight_label(&op))
        .collect())
}

/// Download + install the update stashed by [`release_update_check`].
///
/// Emits [`DownloadProgress`] events on [`DOWNLOAD_EVENT`] for the
/// renderer's progress bar, then installs. On success the stashed
/// handle is cleared. The renderer relaunches the app via
/// `tauri_plugin_process` once this returns `Ok(())`.
///
/// Errors with a surfaced string if no update is stashed (the
/// renderer should always `release_update_check` first).
#[tauri::command]
pub async fn release_update_install(
    app: tauri::AppHandle,
    state: tauri::State<'_, ReleaseUpdateState>,
) -> Result<(), String> {
    // Take the stashed `Update` out of the mutex. `Update` is `Clone`
    // but we MOVE it out rather than clone-and-keep: a successful
    // install invalidates the handle, and a second install attempt
    // against a consumed handle must fail loudly, not silently
    // re-download. On error below we put it back so a retry works.
    let update = {
        let mut guard = state
            .0
            .lock()
            .map_err(|e| format!("update state lock: {e}"))?;
        guard.take()
    };
    let Some(update) = update else {
        return Err("no update is staged — run a check first (release_update_check)".to_string());
    };

    // `app.emit` is best-effort — a failed emit only loses one
    // progress frame, never the install itself. Mirrors the
    // warn-and-swallow discipline of the `op-progress` pipeline.
    let emit = |payload: DownloadProgress| {
        if let Err(e) = app.emit(DOWNLOAD_EVENT, payload) {
            tracing::warn!(
                target = "claudepot_tauri",
                error = %e,
                "release-update download progress emit failed"
            );
        }
    };

    // `download_and_install` takes a FnMut chunk callback and a
    // FnOnce finish callback. The chunk callback reports the chunk
    // length + total; we accumulate the running total ourselves so
    // the renderer gets a cumulative `downloaded` figure.
    let mut downloaded: u64 = 0;
    let mut started = false;
    let install_result = update
        .download_and_install(
            |chunk_len, content_length| {
                if !started {
                    started = true;
                    emit(DownloadProgress::Started { content_length });
                }
                downloaded += chunk_len as u64;
                emit(DownloadProgress::Progress {
                    downloaded,
                    content_length,
                });
            },
            || {
                emit(DownloadProgress::Finished);
            },
        )
        .await;

    match install_result {
        Ok(()) => Ok(()),
        Err(e) => {
            // Put the handle back so the renderer can retry the
            // install without re-running the check.
            *state
                .0
                .lock()
                .map_err(|e| format!("update state lock: {e}"))? = Some(update);
            Err(format!("update install failed: {e}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_channel_endpoints_parse_for_both_channels() {
        // The channel-module endpoints must parse as valid URLs —
        // a typo there would only surface at runtime otherwise.
        for channel in [ReleaseChannel::Stable, ReleaseChannel::Beta] {
            let urls = channel_endpoints(channel).unwrap();
            assert_eq!(urls.len(), 1, "one endpoint per channel today");
            assert_eq!(urls[0].scheme(), "https", "updater endpoints are HTTPS");
        }
    }

    #[test]
    fn test_stable_and_beta_endpoints_differ() {
        let stable = channel_endpoints(ReleaseChannel::Stable).unwrap();
        let beta = channel_endpoints(ReleaseChannel::Beta).unwrap();
        assert_ne!(
            stable[0], beta[0],
            "stable and beta must resolve to distinct manifests"
        );
        assert!(beta[0].as_str().contains("updater-manifest"));
        assert!(stable[0].as_str().contains("/releases/latest/download/"));
    }

    #[test]
    fn test_download_progress_serializes_with_event_tag() {
        // The renderer discriminates on the `event` tag; lock the
        // wire shape so a refactor can't silently rename it.
        let started = serde_json::to_value(DownloadProgress::Started {
            content_length: Some(1024),
        })
        .unwrap();
        assert_eq!(started["event"], "started");
        assert_eq!(started["contentLength"], 1024);

        let progress = serde_json::to_value(DownloadProgress::Progress {
            downloaded: 512,
            content_length: Some(1024),
        })
        .unwrap();
        assert_eq!(progress["event"], "progress");
        assert_eq!(progress["downloaded"], 512);

        let finished = serde_json::to_value(DownloadProgress::Finished).unwrap();
        assert_eq!(finished["event"], "finished");
    }

    #[test]
    fn test_check_dto_serializes_camel_case() {
        let dto = ReleaseUpdateCheckDto {
            update_available: true,
            version: Some("0.2.0-beta.1".to_string()),
            current_version: "0.1.39".to_string(),
            notes: Some("notes".to_string()),
            pub_date: Some("2026-05-21".to_string()),
            channel: "beta".to_string(),
            stranded_on_prerelease: false,
            stable_version: None,
        };
        let v = serde_json::to_value(&dto).unwrap();
        assert_eq!(v["updateAvailable"], true);
        assert_eq!(v["currentVersion"], "0.1.39");
        assert_eq!(v["pubDate"], "2026-05-21");
        assert_eq!(v["channel"], "beta");
        assert_eq!(v["strandedOnPrerelease"], false);
        assert_eq!(v["stableVersion"], serde_json::Value::Null);
    }

    #[test]
    fn test_check_dto_stranded_serializes_stable_version() {
        // The stranded shape the renderer keys on: no update offered,
        // but the stable channel's version is carried for the badge.
        let dto = ReleaseUpdateCheckDto {
            update_available: false,
            version: None,
            current_version: "0.2.0-beta.1".to_string(),
            notes: None,
            pub_date: None,
            channel: "stable".to_string(),
            stranded_on_prerelease: true,
            stable_version: Some("0.1.46".to_string()),
        };
        let v = serde_json::to_value(&dto).unwrap();
        assert_eq!(v["updateAvailable"], false);
        assert_eq!(v["strandedOnPrerelease"], true);
        assert_eq!(v["stableVersion"], "0.1.46");
    }

    #[test]
    fn test_is_stranded_only_when_probe_active_and_remote_not_newer() {
        // Probe inactive (stable build, or Beta channel): never
        // stranded, regardless of what a comparator recorded.
        assert!(!is_stranded(false, None));
        assert!(!is_stranded(false, Some(false)));
        assert!(!is_stranded(false, Some(true)));
        // Probe active, remote strictly newer (stable finally passed
        // the running prerelease): a genuine update, not stranded.
        assert!(!is_stranded(true, Some(true)));
        // Probe active, remote older: the stranded case.
        assert!(is_stranded(true, Some(false)));
        // Probe active but the comparator never recorded (defensive
        // degenerate): treat as a genuine update rather than falsely
        // claiming stranded.
        assert!(!is_stranded(true, None));
    }

    #[test]
    fn test_tauri_conf_updater_endpoints_lock_step_with_stable_endpoint() {
        // `release_channel.rs` documents STABLE_ENDPOINT as
        // byte-identical to `tauri.conf.json`'s
        // `plugins.updater.endpoints`. At runtime the constant always
        // wins — every check overrides via `.endpoints()` and the
        // renderer never calls the JS plugin — so an edit to
        // conf.json alone silently does nothing while looking
        // authoritative. This test forces the two files to move in
        // lock-step.
        let conf: serde_json::Value = serde_json::from_str(include_str!("../../tauri.conf.json"))
            .expect("tauri.conf.json parses as JSON");
        let endpoints: Vec<&str> = conf["plugins"]["updater"]["endpoints"]
            .as_array()
            .expect("plugins.updater.endpoints is an array")
            .iter()
            .map(|v| v.as_str().expect("endpoint is a string"))
            .collect();
        assert_eq!(
            endpoints,
            vec![claudepot_core::release_channel::STABLE_ENDPOINT],
            "tauri.conf.json plugins.updater.endpoints must stay \
             byte-identical to release_channel::STABLE_ENDPOINT"
        );
    }
}
