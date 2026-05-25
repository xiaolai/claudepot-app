//! Permission-grant orchestrator — bridges
//! `claudepot_core::permission` to the Tauri runtime.
//!
//! Unlike `rotation_orchestrator`, this holds **no managed state**:
//! grants live entirely on disk (`~/.claudepot/permission-grants.json`)
//! and are cheap to reload each tick. The orchestrator is two free
//! functions:
//!
//! - [`tick`] — called from `usage_snapshot::run_tick`; reverts every
//!   grant whose deadline has passed and drops it from the file.
//! - [`revert_grant`] — the safety-checked revert, also used directly
//!   by the `permission_revert` command.
//!
//! Zero overhead when no grants exist — `tick` returns after one
//! cheap file read.

use chrono::Utc;
use claudepot_core::breaker;
use claudepot_core::permission::grants::Grant;
use claudepot_core::permission::settings::{
    clear_default_mode, read_default_mode, resolve_default_mode, write_default_mode,
    PermissionSettingsError,
};
use claudepot_core::permission::{eval, store as permission_store};
use serde::Serialize;
use std::path::Path;
use tauri::{AppHandle, Emitter};

/// What happened when a grant's deadline was reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevertOutcome {
    /// The layer still held the granted mode; it was restored to
    /// `previous_mode` (or the key was cleared).
    Reverted,
    /// The layer no longer holds the granted mode — the user changed
    /// the setting themselves after the grant. We don't clobber their
    /// change; the grant is just dropped.
    SkippedUserChanged,
}

/// Revert one grant, with a safety check: only restore `previous_mode`
/// if the settings layer *still* holds exactly `granted_mode`. If the
/// user has since changed the setting by hand, leave their value
/// alone and report [`RevertOutcome::SkippedUserChanged`].
pub fn revert_grant(grant: &Grant) -> Result<RevertOutcome, PermissionSettingsError> {
    let root = Path::new(&grant.project_path);
    let current = read_default_mode(&grant.layer.settings_file(root))?;
    if current.as_ref() != Some(&grant.granted_mode) {
        return Ok(RevertOutcome::SkippedUserChanged);
    }
    match &grant.previous_mode {
        Some(prev) => write_default_mode(grant.layer, root, prev)?,
        None => clear_default_mode(grant.layer, root)?,
    }
    Ok(RevertOutcome::Reverted)
}

/// Drive one expiration cycle. Called from `usage_snapshot::run_tick`
/// after the snapshot is written. Loads grants, reverts the expired
/// ones, and saves the trimmed file. A real I/O failure on load skips
/// the tick (rather than treating it as "no grants" and never
/// reverting); a per-grant revert failure advances that grant's
/// consecutive-failure circuit breaker and leaves it in the file to
/// retry next tick.
///
/// A grant whose breaker is *tripped* — repeated revert failures —
/// is skipped entirely: not reverted, not retried, just left in the
/// file flagged. The breaker's cooldown lets one probe retry through
/// later. The breaker arithmetic is pure `claudepot_core::breaker`;
/// this function only wires it.
pub async fn tick(app: &AppHandle) {
    let mut file = match permission_store::load() {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!(error = %e, "permission_orchestrator: grants load failed; skipping tick");
            return;
        }
    };
    if file.grants.is_empty() {
        return;
    }

    let now = Utc::now();

    // Sweep stale records first: any grant whose `granted_mode` no
    // longer matches the project's LocalProject layer value means the
    // user hand-edited settings since the grant was created. Time-
    // boxed grants self-heal via `revert_grant`'s `skipped_user_changed`
    // path on the expired-revert loop below, but sticky grants would
    // otherwise linger on disk indefinitely. Drop them here so disk
    // state matches what `current_dto::filter_stale` already hides
    // from the UI.
    let stale_paths: Vec<String> = file
        .grants
        .iter()
        .filter(|g| {
            let state = resolve_default_mode(std::path::Path::new(&g.project_path));
            state.local_project_value.as_ref() != Some(&g.granted_mode)
        })
        .map(|g| g.project_path.clone())
        .collect();
    let mut changed = false;
    for path in &stale_paths {
        if file.remove(path).is_some() {
            changed = true;
        }
    }

    let expired_paths: Vec<String> = eval::expired_grants(&file, now)
        .into_iter()
        .map(|g| g.project_path.clone())
        .collect();
    if expired_paths.is_empty() {
        if changed {
            if let Err(e) = permission_store::save(&file) {
                tracing::warn!(error = %e, "permission_orchestrator: stale sweep save failed");
            }
        }
        return;
    }

    for path in &expired_paths {
        // Re-fetch the grant from `file` each iteration so the
        // breaker mutations below land on the live struct.
        let grant = match file.find(path) {
            Some(g) => g.clone(),
            None => continue,
        };

        // Circuit-breaker gate: a grant that has failed to revert
        // THRESHOLD times in a row is quarantined — skip it until
        // the cooldown lets a probe through. Without this, a
        // permanently un-revertable grant (settings file deleted,
        // permission lost) retries every 5 minutes forever.
        if breaker::is_tripped(&grant.breaker_ledger(), now) {
            continue;
        }

        match revert_grant(&grant) {
            Ok(outcome) => {
                // Success removes the grant — its breaker state goes
                // with it. (Probe retries that succeed land here.)
                file.remove(path);
                changed = true;
                emit_reverted(app, &grant, outcome);
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    project = %path,
                    "permission_orchestrator: revert failed; advancing circuit breaker"
                );
                // Advance the breaker on the live grant and persist.
                // `trips_on_next_failure` is checked *before* the
                // record so the trip event fires exactly once, on
                // the failure that crosses the threshold — the same
                // idiom the rotation orchestrator uses.
                if let Some(live) = file.find_mut(path) {
                    let prev = live.breaker_ledger();
                    let newly_tripped = breaker::trips_on_next_failure(&prev);
                    let ledger = breaker::record_failure(&prev, now);
                    live.set_breaker_ledger(ledger);
                    changed = true;
                    if newly_tripped {
                        emit_breaker_tripped(app, &grant, ledger.consecutive);
                    }
                }
            }
        }
    }

    if changed {
        if let Err(e) = permission_store::save(&file) {
            tracing::warn!(error = %e, "permission_orchestrator: grants save failed");
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PermissionRevertedPayload {
    project_path: String,
    /// The mode the project is back to (`previous_mode`, or the CC
    /// default wire string when the grant cleared the key).
    reverted_to: String,
    /// `"reverted"` or `"skipped_user_changed"`.
    outcome: String,
}

fn emit_reverted(app: &AppHandle, grant: &Grant, outcome: RevertOutcome) {
    let reverted_to = grant
        .previous_mode
        .as_ref()
        .map(|m| m.as_wire_str().to_string())
        .unwrap_or_else(|| "default".to_string());
    let payload = PermissionRevertedPayload {
        project_path: grant.project_path.clone(),
        reverted_to,
        outcome: match outcome {
            RevertOutcome::Reverted => "reverted".into(),
            RevertOutcome::SkippedUserChanged => "skipped_user_changed".into(),
        },
    };
    if let Err(e) = app.emit("permission-reverted", payload) {
        tracing::warn!(error = %e, "permission_orchestrator: emit reverted failed");
    }
}

/// `permission-breaker-tripped` payload — a grant's auto-revert kept
/// failing, so its circuit breaker quarantined it. Emitted once, on
/// the failure that crosses the threshold.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PermissionBreakerTrippedPayload {
    project_path: String,
    /// Number of consecutive revert failures that tripped the breaker.
    consecutive_failures: u32,
}

fn emit_breaker_tripped(app: &AppHandle, grant: &Grant, consecutive_failures: u32) {
    let payload = PermissionBreakerTrippedPayload {
        project_path: grant.project_path.clone(),
        consecutive_failures,
    };
    if let Err(e) = app.emit("permission-breaker-tripped", payload) {
        tracing::warn!(error = %e, "permission_orchestrator: emit breaker-tripped failed");
    }
}
