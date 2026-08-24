//! Tauri commands for the Updates panel.
//!
//! State lives in `claudepot_core::updates::UpdateStateMutex`
//! (disk-backed JSON at `~/.claudepot/updates.json`). Long-running
//! install operations block the IPC worker for their duration —
//! brew upgrades and DMG installs are bounded under 5 min, and the
//! UI shows a spinner. If we ever need progress events we can add
//! the op-progress pipeline; for v1 the simpler shape wins.
//!
//! Mirrors the `commands_preferences.rs` pattern: `Option<T>` per
//! settable field so the UI can flip one toggle without re-sending
//! the others.
//!
//! # Error shape
//!
//! Rejections use `ErrorDto` (i18n plan §2.5; `commands/keys.rs` is the
//! reference). `claudepot_core::updates::UpdateError` does not carry an
//! `ErrorCode` yet, so its rejections ride an `updates.*` command code
//! with **core's own English unchanged in `message`** — the same
//! interim shape `templates.instantiate_failed` carries. Promote these
//! to `ErrorDto::from_core` when that enum converts.
//!
//! Two distinctions a translator must keep:
//!
//! - `updates.operation_in_progress` is a **refusal**, not a failure.
//!   Nothing was downloaded, nothing was replaced, and the retry is
//!   immediate — the `PollerGate` exists so a manual click cannot spawn
//!   a second `claude update` beside the poller's.
//! - `updates.unknown_channel` is about **CC's** channel pair
//!   (`latest` / `stable`). Claudepot's own updater has a separate pair
//!   (`stable` / `beta`) under `release_update.unknown_channel`.

use crate::dto_error::{codes, ErrorDto};
use crate::dto_updates::{
    AutoInstallOutcome, CliInstallResultDto, CliStatusDto, DesktopInstallResultDto,
    DesktopStatusDto, UpdatesStatusDto,
};
use claudepot_core::updates::cli_driver::run_claude_update;
use claudepot_core::updates::desktop_driver::install_desktop_latest;
use claudepot_core::updates::poller::{run_one_check_cycle, save_state, PollerGate};
use claudepot_core::updates::{
    count_running_cli_locks, detect_cli_installs, detect_desktop_install, is_desktop_running,
    settings_bridge,
    state::{UpdateSettings, UpdateStateMutex},
    CcChannel, Channel,
};
use serde_json::json;
use std::sync::Arc;

/// Resolve the channel setting from CC's settings.json. CC's setting
/// is the source of truth (see `dev-docs/auto-updates.md` mechanism
/// callout #2).
///
/// Returns the three-state [`CcChannel`], not a `Channel`. Collapsing
/// an unrecognized value to `Latest` is what made the panel light the
/// `latest` button for a user on `rc` and compare their install
/// against the `latest` baseline — see `CcChannel`'s own docs.
fn resolve_channel(cc: &settings_bridge::CcUpdateSettings) -> CcChannel {
    CcChannel::read(cc.auto_updates_channel.as_deref())
}

/// The cached baseline for a channel, or `None` when Claudepot has no
/// feed for it. A version comparison against the wrong channel's
/// baseline is worse than no comparison, so `Untracked` yields
/// nothing rather than the nearest available number.
fn last_known_for(
    channel: &CcChannel,
    cli: &claudepot_core::updates::state::CliCache,
) -> Option<String> {
    match channel.tracked()? {
        Channel::Latest => cli.last_known_latest.clone(),
        Channel::Stable => cli.last_known_stable.clone(),
    }
}

/// The `last_error` an untracked channel deserves — it explains why
/// there is no version to compare against, instead of leaving the row
/// blank or (worse) claiming a network failure that never happened.
fn untracked_channel_note(channel: &CcChannel) -> Option<String> {
    match channel {
        CcChannel::Untracked(raw) => Some(format!(
            "CC is on the {raw:?} update channel, which publishes no version feed — \
             Claudepot cannot tell you whether a newer CC exists on it"
        )),
        _ => None,
    }
}

/// `UpdateStateMutex` is a `std::sync::Mutex`, so a panic under it
/// poisons the lock for every later reader. One spelling for the four
/// call sites that would otherwise each write their own.
fn state_lock_poisoned(e: impl std::fmt::Display) -> ErrorDto {
    ErrorDto::with_params(
        codes::UPDATES_STATE_LOCK_POISONED,
        json!({ "detail": e.to_string() }),
        format!("updates state lock: {e}"),
    )
}

/// The `PollerGate` single-flight refusal. Three call sites shared one
/// English string already; this keeps them sharing one code too.
fn operation_in_progress() -> ErrorDto {
    ErrorDto::new(
        codes::UPDATES_OPERATION_IN_PROGRESS,
        "another update operation is in progress; try again in a moment",
    )
}

/// Read the current snapshot. Pure — no network calls. Safe to call
/// often from the UI for badge refresh.
#[tauri::command]
pub async fn updates_status_get(
    state: tauri::State<'_, UpdateStateMutex>,
) -> Result<UpdatesStatusDto, ErrorDto> {
    let cc = settings_bridge::read().unwrap_or_default();
    let channel = resolve_channel(&cc);
    let installs = detect_cli_installs();
    let desktop = detect_desktop_install();
    let running = is_desktop_running();
    let running_count = count_running_cli_locks();

    let snapshot = state.0.lock().map_err(state_lock_poisoned)?.clone();

    let cli = CliStatusDto {
        channel: channel.label().to_string(),
        installs,
        latest_remote: None, // status_get is offline; check_now refreshes
        last_known: last_known_for(&channel, &snapshot.cache.cli),
        last_check_unix: snapshot.cache.cli.last_check_unix,
        last_error: untracked_channel_note(&channel)
            .or_else(|| snapshot.cache.cli.last_error.clone()),
        cc_settings: cc,
        running_count,
    };
    let desktop = DesktopStatusDto {
        install: desktop,
        running,
        latest_remote: snapshot.cache.desktop.last_known_latest.clone(),
        latest_commit_sha: snapshot.cache.desktop.last_known_sha.clone(),
        last_check_unix: snapshot.cache.desktop.last_check_unix,
        last_error: snapshot.cache.desktop.last_error.clone(),
    };
    Ok(UpdatesStatusDto {
        cli,
        desktop,
        settings: snapshot.settings,
        cli_auto_outcome: AutoInstallOutcome::Disabled,
        desktop_auto_outcome: AutoInstallOutcome::Disabled,
    })
}

/// Force a fresh probe of upstream for both CC and Desktop, persist
/// the result, run the auto-install pass if flags allow, and return
/// the refreshed status.
///
/// Delegates the policy to `claudepot_core::updates::poller::
/// run_one_check_cycle` so the background poller and this command
/// share a single implementation. Failure paths are captured in the
/// outcome — the command itself only errors on lock-contention or
/// state-deserialization issues, never on network or install
/// failures.
///
/// Single-flights against the `PollerGate` so a manual click can't
/// race the background poller (or another manual click) — which
/// would otherwise spawn duplicate `claude update` / `brew upgrade`
/// subprocesses.
#[tauri::command]
pub async fn updates_check_now(
    state: tauri::State<'_, UpdateStateMutex>,
    gate: tauri::State<'_, Arc<PollerGate>>,
) -> Result<UpdatesStatusDto, ErrorDto> {
    let arc_gate: Arc<PollerGate> = (*gate).clone();
    let _lease = arc_gate.try_acquire().ok_or_else(operation_in_progress)?;

    let outcome = run_one_check_cycle(&state).await;
    save_state(&state).await;

    // Re-collect the snapshot post-cycle. Cheaper to call the status
    // getter than to maintain two parallel return shapes.
    let mut dto = build_status(&state)?;
    dto.cli.latest_remote = outcome.cli_latest.clone();
    dto.desktop.latest_remote = outcome.desktop_latest.clone();
    dto.desktop.latest_commit_sha = outcome.desktop_latest_sha.clone();
    dto.cli_auto_outcome = outcome.cli_auto;
    dto.desktop_auto_outcome = outcome.desktop_auto;
    Ok(dto)
}

/// Build the status DTO from current state without going through
/// the IPC entrypoint. Lets `updates_check_now` reuse
/// `updates_status_get`'s shape without recursive State borrow.
fn build_status(state: &UpdateStateMutex) -> Result<UpdatesStatusDto, ErrorDto> {
    let cc = settings_bridge::read().unwrap_or_default();
    let channel = resolve_channel(&cc);
    let installs = detect_cli_installs();
    let desktop = detect_desktop_install();
    let running = is_desktop_running();
    let running_count = count_running_cli_locks();

    let snapshot = state.0.lock().map_err(state_lock_poisoned)?.clone();

    let cli = CliStatusDto {
        channel: channel.label().to_string(),
        installs,
        latest_remote: None,
        last_known: last_known_for(&channel, &snapshot.cache.cli),
        last_check_unix: snapshot.cache.cli.last_check_unix,
        last_error: untracked_channel_note(&channel)
            .or_else(|| snapshot.cache.cli.last_error.clone()),
        cc_settings: cc,
        running_count,
    };
    let desktop = DesktopStatusDto {
        install: desktop,
        running,
        latest_remote: snapshot.cache.desktop.last_known_latest.clone(),
        latest_commit_sha: snapshot.cache.desktop.last_known_sha.clone(),
        last_check_unix: snapshot.cache.desktop.last_check_unix,
        last_error: snapshot.cache.desktop.last_error.clone(),
    };
    Ok(UpdatesStatusDto {
        cli,
        desktop,
        settings: snapshot.settings,
        cli_auto_outcome: AutoInstallOutcome::Disabled,
        desktop_auto_outcome: AutoInstallOutcome::Disabled,
    })
}

/// Force-run `claude update`. Refuses if `DISABLE_UPDATES=1` or if
/// another update operation (background poller, another manual
/// click) is already in flight.
#[tauri::command]
pub async fn updates_cli_install(
    gate: tauri::State<'_, Arc<PollerGate>>,
) -> Result<CliInstallResultDto, ErrorDto> {
    let arc_gate: Arc<PollerGate> = (*gate).clone();
    let _lease = arc_gate.try_acquire().ok_or_else(operation_in_progress)?;
    let outcome = run_claude_update()
        .await
        .map_err(|e| ErrorDto::detail(codes::UPDATES_CLI_INSTALL_FAILED, e))?;
    Ok(CliInstallResultDto {
        stdout: outcome.stdout,
        stderr: outcome.stderr,
        installed_after: outcome.installed_after,
    })
}

/// Drive a Desktop install. Refuses if Desktop is currently running
/// or if another update operation is already in flight.
#[tauri::command]
pub async fn updates_desktop_install(
    gate: tauri::State<'_, Arc<PollerGate>>,
) -> Result<DesktopInstallResultDto, ErrorDto> {
    let arc_gate: Arc<PollerGate> = (*gate).clone();
    let _lease = arc_gate.try_acquire().ok_or_else(operation_in_progress)?;
    let outcome = install_desktop_latest()
        .await
        .map_err(|e| ErrorDto::detail(codes::UPDATES_DESKTOP_INSTALL_FAILED, e))?;
    Ok(DesktopInstallResultDto {
        method: outcome.method,
        version_after: outcome.version_after,
        stdout: outcome.stdout,
        stderr: outcome.stderr,
    })
}

/// Read the current settings (Claudepot side only — CC settings flow
/// through `updates_status_get`).
#[tauri::command]
pub async fn updates_settings_get(
    state: tauri::State<'_, UpdateStateMutex>,
) -> Result<UpdateSettings, ErrorDto> {
    Ok(state
        .0
        .lock()
        .map_err(state_lock_poisoned)?
        .settings
        .clone())
}

/// Mutate one or more settings fields. Mirrors the
/// `preferences_set_*` pattern: `Option<T>` per field, only the
/// `Some(_)` ones are written.
///
/// Routes the disk write through `save_state` so it serializes
/// against the watcher's cycle save (via the shared `save_lock`).
/// Without that, a settings toggle could land an in-memory write
/// while the watcher's older snapshot is still being written to
/// disk — losing the toggle.
// One arg per setting — Tauri commands deserialize each named arg
// from the JS side, so a struct here would force the renderer to
// nest the patch one level deeper. Same rationale as
// preferences_set_notifications; allow per-function.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn updates_settings_set(
    state: tauri::State<'_, UpdateStateMutex>,
    cli_notify_on_available: Option<bool>,
    cli_notify_os_on_available: Option<bool>,
    cli_force_update_on_check: Option<bool>,
    desktop_notify_on_available: Option<bool>,
    desktop_notify_os_on_available: Option<bool>,
    desktop_auto_install_when_quit: Option<bool>,
    poll_interval_minutes: Option<Option<u32>>,
) -> Result<UpdateSettings, ErrorDto> {
    let settings = {
        let mut guard = state.0.lock().map_err(state_lock_poisoned)?;
        if let Some(v) = cli_notify_on_available {
            guard.settings.cli.notify_on_available = v;
        }
        if let Some(v) = cli_notify_os_on_available {
            guard.settings.cli.notify_os_on_available = v;
        }
        if let Some(v) = cli_force_update_on_check {
            guard.settings.cli.force_update_on_check = v;
        }
        if let Some(v) = desktop_notify_on_available {
            guard.settings.desktop.notify_on_available = v;
        }
        if let Some(v) = desktop_notify_os_on_available {
            guard.settings.desktop.notify_os_on_available = v;
        }
        if let Some(v) = desktop_auto_install_when_quit {
            guard.settings.desktop.auto_install_when_quit = v;
        }
        if let Some(v) = poll_interval_minutes {
            guard.settings.poll_interval_minutes = v;
        }
        guard.settings.clone()
    };
    save_state(&state).await;
    Ok(settings)
}

/// Set CC's release channel. Writes to `~/.claude/settings.json`
/// (CC's file, NOT Claudepot's).
///
/// **`allow_downgrade`** mirrors CC's `/config` UX for the
/// `latest → stable` transition: `false` (default) pins
/// `minimumVersion` to the currently-installed version so the user
/// isn't involuntarily downgraded; `true` clears the floor and
/// accepts the downgrade. Ignored for other transitions.
///
/// Passing `None` for `channel` clears both `autoUpdatesChannel`
/// AND `minimumVersion` — leaving a stale floor behind would
/// silently constrain a user who explicitly reverted to CC's
/// defaults.
#[tauri::command]
pub async fn updates_channel_set(
    channel: Option<String>,
    allow_downgrade: Option<bool>,
) -> Result<(), ErrorDto> {
    // Clearing the channel is one transition over two keys, so both
    // writes carry the same code — the user pressed one button, and a
    // failure in either half leaves the same half-applied state.
    let target = match channel.as_deref() {
        None => {
            settings_bridge::write_minimum_version(None).map_err(channel_write_failed)?;
            return settings_bridge::write_channel(None).map_err(channel_write_failed);
        }
        Some(c) => c,
    };
    // Core's `Display` is kept verbatim; `params.channel` is what a
    // localized sentence names instead of re-parsing the quotes back
    // out of the English.
    target.parse::<Channel>().map_err(|e| {
        ErrorDto::with_params(
            codes::UPDATES_UNKNOWN_CHANNEL,
            json!({ "channel": target, "detail": e.to_string() }),
            e.to_string(),
        )
    })?;

    let installed = detect_cli_installs()
        .into_iter()
        .find(|c| c.is_active)
        .and_then(|c| c.version);
    settings_bridge::change_channel(
        target,
        installed.as_deref(),
        allow_downgrade.unwrap_or(false),
    )
    .map_err(channel_write_failed)?;
    Ok(())
}

/// One code for every write that moves CC's `autoUpdatesChannel` —
/// including the `minimumVersion` half of the same transition.
fn channel_write_failed(e: impl std::fmt::Display) -> ErrorDto {
    ErrorDto::detail(codes::UPDATES_CHANNEL_WRITE_FAILED, e)
}

/// Set CC's `minimumVersion` floor. Writes to `~/.claude/settings.json`.
#[tauri::command]
pub async fn updates_minimum_version_set(version: Option<String>) -> Result<(), ErrorDto> {
    settings_bridge::write_minimum_version(version.as_deref())
        .map_err(|e| ErrorDto::detail(codes::UPDATES_MINIMUM_VERSION_WRITE_FAILED, e))
}
