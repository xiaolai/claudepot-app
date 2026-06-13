//! Tauri commands for the per-project permission surface.
//!
//! All async per `commands/mod.rs` threading policy. The pure logic
//! (mode parsing, grant schema, expiration eval, settings RMW) lives
//! in `claudepot_core::permission`; this module marshals DTOs and
//! runs the (tiny) file I/O off the main thread.

use chrono::{Duration, Utc};
use claudepot_core::permission::grants::Grant;
use claudepot_core::permission::settings::resolve_default_mode;
use claudepot_core::permission::{
    eval, store as permission_store, write_default_mode, PermissionMode,
};
use claudepot_core::project;
use claudepot_core::settings_writer::SettingsLayer;
use std::path::Path;

use super::validate_project_path;
use crate::dto_permission::{project_permission_dto, ProjectPermissionDto};
use crate::permission_orchestrator::revert_grant;

/// Grant durations the UI offers sit well inside this range. The
/// bounds are a guard rail against a malformed call, not a policy —
/// `permission_grant` rejects anything outside it loudly.
const MIN_DURATION_SECS: u64 = 60;
const MAX_DURATION_SECS: u64 = 24 * 60 * 60;

fn validate_duration(secs: u64) -> Result<i64, String> {
    if !(MIN_DURATION_SECS..=MAX_DURATION_SECS).contains(&secs) {
        return Err(format!(
            "duration must be {MIN_DURATION_SECS}..={MAX_DURATION_SECS} seconds, got {secs}"
        ));
    }
    // In-range against a 24h ceiling — the i64 cast cannot overflow.
    Ok(secs as i64)
}

/// Resolve a renderer-supplied `duration_secs` (None = sticky) into
/// an `expires_at` deadline. Centralized so `permission_grant` and
/// `permission_extend` honor the same rule: `None` → sticky grant
/// (no deadline); `Some(n)` → must lie in `[MIN, MAX]` seconds.
fn resolve_expires_at(
    duration_secs: Option<u64>,
    now: chrono::DateTime<Utc>,
) -> Result<Option<chrono::DateTime<Utc>>, String> {
    match duration_secs {
        None => Ok(None),
        Some(secs) => {
            let seconds = validate_duration(secs)?;
            Ok(Some(now + Duration::seconds(seconds)))
        }
    }
}

/// Load the grants file under the explicit three-outcome contract
/// ([`permission_store::load_outcome`]) — every store read in this
/// module routes through here. A corruption recovery returns the
/// recovered (empty) file; the user-visible "elevated projects may
/// not auto-revert" notice is owned by
/// `permission_orchestrator::tick`, which detects recoveries from
/// any surface via `corrupt_grant_copies`.
fn load_grants() -> Result<claudepot_core::permission::grants::GrantsFile, String> {
    permission_store::load_outcome()
        .map(|loaded| loaded.value)
        .map_err(|e| format!("grants load failed: {e}"))
}

/// Resolve `project_path` to a `ProjectPermissionDto`, reading the
/// current settings state and the active grant (if any) from disk.
fn current_dto(project_path: &str) -> ProjectPermissionDto {
    let state = resolve_default_mode(Path::new(project_path));
    let file = permission_store::load_or_default();
    let active = eval::active_grant(&file, project_path, Utc::now());
    let active = filter_stale(&state, active);
    project_permission_dto(project_path.to_string(), &state, active)
}

/// Drop the grant from the DTO when the LocalProject layer's value
/// no longer matches `granted_mode` — the user has hand-edited
/// settings since the grant was created, so we're no longer managing
/// the project's permission state.
///
/// Critical for sticky grants: a time-boxed grant self-heals on the
/// orchestrator's next tick (via `revert_grant`'s
/// `skipped_user_changed` path), but a sticky grant's expiration
/// path never fires. Without this filter, a stale sticky grant
/// would surface as "Bypass active" forever even after the user
/// removed the elevation by hand.
fn filter_stale<'a>(
    state: &claudepot_core::permission::settings::PermissionState,
    active: Option<&'a Grant>,
) -> Option<&'a Grant> {
    active.filter(|g| state.local_project_value.as_ref() == Some(&g.granted_mode))
}

/// Every CC project with its effective permission mode and any active
/// Claudepot grant. The dashboard's data source.
#[tauri::command]
pub async fn permission_list() -> Result<Vec<ProjectPermissionDto>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let cfg = claudepot_core::paths::claude_config_dir();
        let projects = project::list_projects(&cfg).map_err(|e| format!("list failed: {e}"))?;
        // `load_outcome` (not `load_or_default`) so a real I/O failure
        // surfaces instead of silently rendering every project as
        // un-granted. A corruption recovery (file moved aside, empty
        // grants returned) is user-surfaced by the permission
        // orchestrator's corruption notice — `corrupt_grant_copies`
        // makes the recovery visible to its scan — so here the
        // recovered file is simply the best available truth.
        let file = load_grants()?;
        let now = Utc::now();
        Ok(projects
            .iter()
            .map(|p| {
                let state = resolve_default_mode(Path::new(&p.original_path));
                let active = eval::active_grant(&file, &p.original_path, now);
                let active = filter_stale(&state, active);
                project_permission_dto(p.original_path.clone(), &state, active)
            })
            .collect())
    })
    .await
    .map_err(|e| format!("permission_list join: {e}"))?
}

/// One project's permission state. The single-project sibling of
/// [`permission_list`] — used by the ProjectDetail panel so opening a
/// project doesn't trigger a full project-tree scan.
#[tauri::command]
pub async fn permission_get(project_path: String) -> Result<ProjectPermissionDto, String> {
    tauri::async_runtime::spawn_blocking(move || {
        // Three-outcome load — see `load_grants` for the recovery
        // contract.
        let file = load_grants()?;
        let state = resolve_default_mode(Path::new(&project_path));
        let active = eval::active_grant(&file, &project_path, Utc::now());
        let active = filter_stale(&state, active);
        Ok(project_permission_dto(project_path.clone(), &state, active))
    })
    .await
    .map_err(|e| format!("permission_get join: {e}"))?
}

/// Set `permissions.defaultMode` for a project to `mode` for
/// `duration_secs`, recording a grant the orchestrator auto-reverts.
/// Re-granting a project that already has a grant preserves the
/// *original* `previous_mode` so revert still restores the true
/// pre-Claudepot state.
// `duration_secs`: `None` → sticky grant (no auto-revert);
// `Some(secs)` → time-boxed, must lie in the `validate_duration`
// range.
//
// Wire-contract note: a missing `durationSecs` JSON key
// deserializes the same as an explicit `null` (both → sticky).
// This is acceptable under the IPC trust model documented in
// `.claude/rules/architecture.md` ("Tauri 2 IPC is in-process — JS
// bridge is not a cross-trust boundary"): the renderer is our own
// code and the TS API (`src/api/permission.ts`) types the field as
// `number | null`, forcing typed call sites to be explicit. A
// hand-coded `invoke()` that omits the field would be a renderer
// bug caught in review, not an exploit vector.
#[tauri::command]
pub async fn permission_grant(
    project_path: String,
    mode: String,
    duration_secs: Option<u64>,
) -> Result<ProjectPermissionDto, String> {
    let granted_mode = PermissionMode::from_wire_str(&mode);
    if !granted_mode.is_known() {
        return Err(format!(
            "`{mode}` is not a permission mode Claudepot can grant"
        ));
    }
    let now = Utc::now();
    let expires_at = resolve_expires_at(duration_secs, now)?;

    tauri::async_runtime::spawn_blocking(move || {
        validate_project_path(&project_path)?;
        let root = Path::new(&project_path);
        // Hold the grants-file lock across the whole load → mutate →
        // save (and the settings write that must stay consistent with
        // it) so an orchestrator tick can't save an older snapshot
        // over this grant. See `permission_orchestrator::grants_file_guard`.
        let _guard = crate::permission_orchestrator::grants_file_guard();
        let mut file = load_grants()?;

        // Preserve the true original mode across a re-grant: if a
        // grant already exists, its `previous_mode` is the real
        // pre-Claudepot value — capturing the layer's *current* value
        // now would just record the prior grant's mode.
        let previous_mode = match file.find(&project_path) {
            Some(existing) => existing.previous_mode.clone(),
            None => resolve_default_mode(root).local_project_value,
        };

        let grant = Grant {
            project_path: project_path.clone(),
            layer: SettingsLayer::LocalProject,
            granted_mode: granted_mode.clone(),
            previous_mode,
            granted_at: now,
            expires_at,
            // Fresh grant — its revert circuit breaker starts clean.
            consecutive_failures: 0,
            last_failure_at: None,
        };

        // Persist the grant record FIRST. If this fails the settings
        // file is untouched — a clean failure with nothing to undo.
        file.upsert(grant);
        permission_store::save(&file).map_err(|e| format!("grants save failed: {e}"))?;

        // Then write the settings. If THIS fails, roll the grant
        // record back out — otherwise the project would be left
        // elevated with no managing grant, which the orchestrator
        // would never revert. (Even if the rollback save also fails,
        // the orchestrator self-heals: `revert_grant` sees the layer
        // never held `granted_mode` and drops the grant.)
        if let Err(e) = write_default_mode(SettingsLayer::LocalProject, root, &granted_mode) {
            file.remove(&project_path);
            let _ = permission_store::save(&file);
            return Err(format!("settings write failed: {e}"));
        }

        Ok(current_dto(&project_path))
    })
    .await
    .map_err(|e| format!("permission_grant join: {e}"))?
}

/// Revert a project's grant immediately — restore `previous_mode`
/// (or clear the key) and drop the grant. Errors if the project has
/// no active grant; a project elevated by hand-editing settings is
/// not Claudepot-managed and is left untouched.
#[tauri::command]
pub async fn permission_revert(project_path: String) -> Result<ProjectPermissionDto, String> {
    tauri::async_runtime::spawn_blocking(move || {
        // Serialize against the orchestrator tick — see
        // `permission_orchestrator::grants_file_guard`.
        let _guard = crate::permission_orchestrator::grants_file_guard();
        let mut file = load_grants()?;
        let grant = file
            .find(&project_path)
            .cloned()
            .ok_or_else(|| format!("no active grant for {project_path}"))?;

        revert_grant(&grant).map_err(|e| format!("revert failed: {e}"))?;
        file.remove(&project_path);
        permission_store::save(&file).map_err(|e| format!("grants save failed: {e}"))?;

        Ok(current_dto(&project_path))
    })
    .await
    .map_err(|e| format!("permission_revert join: {e}"))?
}

/// Update a grant's deadline. `Some(secs)` pushes the deadline out
/// to `secs` from now (time-boxed); `None` converts the grant to
/// **sticky** — no auto-revert. Errors if the project has no active
/// grant.
// `duration_secs`: `None` → convert to sticky (no deadline);
// `Some(secs)` → push deadline out from now.
#[tauri::command]
pub async fn permission_extend(
    project_path: String,
    duration_secs: Option<u64>,
) -> Result<ProjectPermissionDto, String> {
    let now = Utc::now();
    let expires_at = resolve_expires_at(duration_secs, now)?;
    tauri::async_runtime::spawn_blocking(move || {
        // Serialize against the orchestrator tick — see
        // `permission_orchestrator::grants_file_guard`.
        let _guard = crate::permission_orchestrator::grants_file_guard();
        let mut file = load_grants()?;
        let grant = file
            .grants
            .iter_mut()
            .find(|g| g.project_path == project_path)
            .ok_or_else(|| format!("no active grant for {project_path}"))?;
        grant.expires_at = expires_at;
        permission_store::save(&file).map_err(|e| format!("grants save failed: {e}"))?;

        Ok(current_dto(&project_path))
    })
    .await
    .map_err(|e| format!("permission_extend join: {e}"))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use claudepot_core::permission::grants::Grant;
    use claudepot_core::permission::settings::{PermissionDecisionSource, PermissionState};
    use claudepot_core::permission::PermissionMode;
    use claudepot_core::settings_writer::SettingsLayer;

    fn state_with_local(value: Option<PermissionMode>) -> PermissionState {
        PermissionState {
            effective: value.clone().unwrap_or(PermissionMode::Default),
            decided_by: PermissionDecisionSource::LocalProjectSettings,
            local_project_value: value,
            project_value: None,
            user_value: None,
        }
    }

    fn sticky_grant(granted: PermissionMode) -> Grant {
        Grant {
            project_path: "/p/a".into(),
            layer: SettingsLayer::LocalProject,
            granted_mode: granted,
            previous_mode: Some(PermissionMode::Default),
            granted_at: Utc::now(),
            expires_at: None,
            consecutive_failures: 0,
            last_failure_at: None,
        }
    }

    #[test]
    fn filter_stale_keeps_grant_when_layer_matches() {
        let g = sticky_grant(PermissionMode::BypassPermissions);
        let state = state_with_local(Some(PermissionMode::BypassPermissions));
        assert!(filter_stale(&state, Some(&g)).is_some());
    }

    #[test]
    fn filter_stale_drops_grant_when_layer_diverges() {
        // User hand-edited settings.local.json to plain `default` —
        // the sticky grant record is now lying. We must NOT surface
        // the grant in the DTO; that's the "stays elevated forever"
        // bug Codex flagged.
        let g = sticky_grant(PermissionMode::BypassPermissions);
        let state = state_with_local(Some(PermissionMode::Default));
        assert!(filter_stale(&state, Some(&g)).is_none());
    }

    #[test]
    fn filter_stale_drops_grant_when_layer_cleared() {
        // User removed the key entirely. Same defect class as above.
        let g = sticky_grant(PermissionMode::BypassPermissions);
        let state = state_with_local(None);
        assert!(filter_stale(&state, Some(&g)).is_none());
    }

    #[test]
    fn filter_stale_passes_through_no_grant() {
        let state = state_with_local(None);
        assert!(filter_stale(&state, None).is_none());
    }
}
