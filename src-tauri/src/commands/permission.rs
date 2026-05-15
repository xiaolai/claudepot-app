//! Tauri commands for the per-project permission surface.
//!
//! All async per `commands/mod.rs` threading policy. The pure logic
//! (mode parsing, grant schema, expiration eval, settings RMW) lives
//! in `claudepot_core::permission`; this module marshals DTOs and
//! runs the (tiny) file I/O off the main thread.

use chrono::{Duration, Utc};
use claudepot_core::permission::grants::Grant;
use claudepot_core::permission::settings::resolve_default_mode;
use claudepot_core::permission::{eval, store as permission_store, write_default_mode, PermissionMode};
use claudepot_core::project;
use claudepot_core::settings_writer::SettingsLayer;
use std::path::Path;

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

/// Resolve `project_path` to a `ProjectPermissionDto`, reading the
/// current settings state and the active grant (if any) from disk.
fn current_dto(project_path: &str) -> ProjectPermissionDto {
    let state = resolve_default_mode(Path::new(project_path));
    let file = permission_store::load_or_default();
    let active = eval::active_grant(&file, project_path, Utc::now());
    project_permission_dto(project_path.to_string(), &state, active)
}

/// Every CC project with its effective permission mode and any active
/// Claudepot grant. The dashboard's data source.
#[tauri::command]
pub async fn permission_list() -> Result<Vec<ProjectPermissionDto>, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let cfg = claudepot_core::paths::claude_config_dir();
        let projects =
            project::list_projects(&cfg).map_err(|e| format!("list failed: {e}"))?;
        // `load` (not `load_or_default`) so a real I/O failure surfaces
        // instead of silently rendering every project as un-granted.
        let file = permission_store::load().map_err(|e| format!("grants load failed: {e}"))?;
        let now = Utc::now();
        Ok(projects
            .iter()
            .map(|p| {
                let state = resolve_default_mode(Path::new(&p.original_path));
                let active = eval::active_grant(&file, &p.original_path, now);
                project_permission_dto(p.original_path.clone(), &state, active)
            })
            .collect())
    })
    .await
    .map_err(|e| format!("permission_list join: {e}"))?
}

/// Set `permissions.defaultMode` for a project to `mode` for
/// `duration_secs`, recording a grant the orchestrator auto-reverts.
/// Re-granting a project that already has a grant preserves the
/// *original* `previous_mode` so revert still restores the true
/// pre-Claudepot state.
#[tauri::command]
pub async fn permission_grant(
    project_path: String,
    mode: String,
    duration_secs: u64,
) -> Result<ProjectPermissionDto, String> {
    let granted_mode = PermissionMode::from_wire_str(&mode);
    if !granted_mode.is_known() {
        return Err(format!("`{mode}` is not a permission mode Claudepot can grant"));
    }
    let duration = validate_duration(duration_secs)?;

    tauri::async_runtime::spawn_blocking(move || {
        let root = Path::new(&project_path);
        let mut file =
            permission_store::load().map_err(|e| format!("grants load failed: {e}"))?;

        // Preserve the true original mode across a re-grant: if a
        // grant already exists, its `previous_mode` is the real
        // pre-Claudepot value — capturing the layer's *current* value
        // now would just record the prior grant's mode.
        let previous_mode = match file.find(&project_path) {
            Some(existing) => existing.previous_mode.clone(),
            None => resolve_default_mode(root).local_project_value,
        };

        write_default_mode(SettingsLayer::LocalProject, root, &granted_mode)
            .map_err(|e| format!("settings write failed: {e}"))?;

        let now = Utc::now();
        let grant = Grant {
            project_path: project_path.clone(),
            layer: SettingsLayer::LocalProject,
            granted_mode,
            previous_mode,
            granted_at: now,
            expires_at: now + Duration::seconds(duration),
        };
        file.upsert(grant);
        permission_store::save(&file).map_err(|e| format!("grants save failed: {e}"))?;

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
        let mut file =
            permission_store::load().map_err(|e| format!("grants load failed: {e}"))?;
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

/// Push a grant's deadline out to `duration_secs` from now. Errors if
/// the project has no active grant.
#[tauri::command]
pub async fn permission_extend(
    project_path: String,
    duration_secs: u64,
) -> Result<ProjectPermissionDto, String> {
    let duration = validate_duration(duration_secs)?;
    tauri::async_runtime::spawn_blocking(move || {
        let mut file =
            permission_store::load().map_err(|e| format!("grants load failed: {e}"))?;
        let grant = file
            .grants
            .iter_mut()
            .find(|g| g.project_path == project_path)
            .ok_or_else(|| format!("no active grant for {project_path}"))?;
        grant.expires_at = Utc::now() + Duration::seconds(duration);
        permission_store::save(&file).map_err(|e| format!("grants save failed: {e}"))?;

        Ok(current_dto(&project_path))
    })
    .await
    .map_err(|e| format!("permission_extend join: {e}"))?
}
