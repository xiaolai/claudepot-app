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
use claudepot_core::permission::grants::Grant;
use claudepot_core::permission::settings::{
    clear_default_mode, read_default_mode, write_default_mode, PermissionSettingsError,
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
/// reverting); a per-grant revert failure leaves that grant in the
/// file to retry next tick.
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
    let expired: Vec<Grant> = eval::expired_grants(&file, now)
        .into_iter()
        .cloned()
        .collect();
    if expired.is_empty() {
        return;
    }

    let mut changed = false;
    for grant in &expired {
        match revert_grant(grant) {
            Ok(outcome) => {
                file.remove(&grant.project_path);
                changed = true;
                emit_reverted(app, grant, outcome);
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    project = %grant.project_path,
                    "permission_orchestrator: revert failed; leaving grant for next tick"
                );
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
