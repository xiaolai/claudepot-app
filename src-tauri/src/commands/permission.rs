//! Tauri commands for the per-project permission surface.
//!
//! All async per `commands/mod.rs` threading policy. The pure logic
//! (mode parsing, grant schema, expiration eval, hook decision, hook
//! installation) lives in `claudepot_core::permission`; this module
//! marshals DTOs and runs the (tiny) file I/O off the main thread.
//!
//! A grant writes nothing into Claude Code's settings except the hook
//! entry that answers for it — see `claudepot_core::permission::hook`
//! for why, and `permission_orchestrator` for what keeps that entry
//! honest between commands.

use chrono::{Duration, Utc};
use claudepot_core::permission::grants::Grant;
use claudepot_core::permission::settings::resolve_default_mode;
use claudepot_core::permission::{
    clear_default_mode, eval, hook, store as permission_store, PermissionState,
};
use claudepot_core::project;
use claudepot_core::settings_writer::SettingsLayer;
use std::path::Path;

use super::validate_project_path;
use crate::dto_error::{codes, ErrorDto};
use crate::dto_permission::{project_permission_dto, ProjectPermissionDto};
use crate::permission_orchestrator::{grants_file_guard, reconcile_hook};

/// Grant durations the UI offers sit well inside this range. The
/// bounds are a guard rail against a malformed call, not a policy —
/// `permission_grant` rejects anything outside it loudly.
const MIN_DURATION_SECS: u64 = 60;
const MAX_DURATION_SECS: u64 = 24 * 60 * 60;

fn validate_duration(secs: u64) -> Result<i64, ErrorDto> {
    if !(MIN_DURATION_SECS..=MAX_DURATION_SECS).contains(&secs) {
        return Err(ErrorDto::with_params(
            codes::PERMISSION_DURATION_OUT_OF_RANGE,
            serde_json::json!({
                "min": MIN_DURATION_SECS,
                "max": MAX_DURATION_SECS,
                "got": secs,
            }),
            format!(
                "duration must be {MIN_DURATION_SECS}..={MAX_DURATION_SECS} seconds, got {secs}"
            ),
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
) -> Result<Option<chrono::DateTime<Utc>>, ErrorDto> {
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
/// recovered (empty) file; the user-visible notice is owned by
/// `permission_orchestrator::tick`.
fn load_grants() -> Result<claudepot_core::permission::grants::GrantsFile, ErrorDto> {
    permission_store::load_outcome()
        .map(|loaded| loaded.value)
        .map_err(|e| ErrorDto::detail(codes::PERMISSION_GRANTS_LOAD_FAILED, e))
}

/// Is CC's `PreToolUse` entry present and pointing at the CLI binary
/// the orchestrator installs (`hook::hook_binary`, never this GUI
/// executable)? Read once per command, not per project row. A
/// resolver failure reads as "not installed", which is the honest
/// answer: nothing could have installed it either.
fn hook_installed() -> bool {
    hook::hook_binary()
        .ok()
        .and_then(|bin| hook::installed_state(&bin))
        .unwrap_or(false)
}

fn no_active_grant(project_path: &str) -> ErrorDto {
    ErrorDto::with_params(
        codes::PERMISSION_NO_ACTIVE_GRANT,
        serde_json::json!({ "project_path": project_path }),
        format!("no active grant for {project_path}"),
    )
}

/// Resolve `project_path` to a `ProjectPermissionDto`, reading the
/// current settings state and the active grant (if any) from disk.
fn current_dto(project_path: &str) -> Result<ProjectPermissionDto, ErrorDto> {
    let state = resolve_default_mode(Path::new(project_path)).map_err(ErrorDto::from)?;
    let file = permission_store::load_or_default();
    let active = eval::active_grant(&file, project_path, Utc::now());
    Ok(project_permission_dto(
        project_path.to_string(),
        &state,
        active,
        hook_installed(),
    ))
}

/// Persist `file`, then make the hook entry agree with it. Both under
/// the caller's [`grants_file_guard`]. A grant whose hook cannot be
/// installed is exactly the silent no-op this feature replaced, so a
/// failed reconcile after a *grant* is an error the caller must
/// surface rather than a warning in a log nobody reads.
fn save_and_reconcile(
    file: &claudepot_core::permission::grants::GrantsFile,
) -> Result<bool, ErrorDto> {
    permission_store::save(file).map_err(ErrorDto::from)?;
    reconcile_hook(file, Utc::now())
        .map_err(|detail| ErrorDto::detail(codes::PERMISSION_HOOK_INSTALL_FAILED, detail))
}

/// Run `persist` over a file that `mutate` has just changed; if it
/// fails, put the file back the way `undo` says and persist *that*,
/// so a failed command never leaves grant state it did not report.
///
/// A re-grant used to roll back by removing the grant, which threw
/// away the grant that was there before; an extend used to save its
/// new deadline and then fail, keeping the deadline. Both are the
/// same mistake — a rollback that restores a guessed state rather
/// than the prior one — so the shape is written once. The undo's own
/// save is best effort: if the disk is failing the caller already has
/// the error, and the orchestrator's next tick re-reconciles.
fn commit_or_rollback<T>(
    file: &mut claudepot_core::permission::grants::GrantsFile,
    undo: impl FnOnce(&mut claudepot_core::permission::grants::GrantsFile),
    persist: impl FnOnce(&claudepot_core::permission::grants::GrantsFile) -> Result<T, ErrorDto>,
    save_rollback: impl FnOnce(&claudepot_core::permission::grants::GrantsFile),
) -> Result<T, ErrorDto> {
    match persist(file) {
        Ok(v) => Ok(v),
        Err(e) => {
            undo(file);
            save_rollback(file);
            Err(e)
        }
    }
}

/// The production `save_rollback`: best effort, see
/// [`commit_or_rollback`]. Tests inject a recorder instead, so a unit
/// test of the rollback never writes to a real `CLAUDEPOT_DATA_DIR`.
fn save_rollback_to_disk(file: &claudepot_core::permission::grants::GrantsFile) {
    if let Err(e) = permission_store::save(file) {
        tracing::warn!(error = %e, "permission: rollback save failed");
    }
}

/// Every CC project with its effective permission mode and any active
/// Claudepot grant. The dashboard's data source.
#[tauri::command]
pub async fn permission_list() -> Result<Vec<ProjectPermissionDto>, ErrorDto> {
    tauri::async_runtime::spawn_blocking(|| {
        let cfg = claudepot_core::paths::claude_config_dir();
        let projects = project::list_projects(&cfg).map_err(ErrorDto::from)?;
        let file = load_grants()?;
        let now = Utc::now();
        let installed = hook_installed();
        projects
            .iter()
            .map(|p| -> Result<_, ErrorDto> {
                // The one place the prefix stays in Rust: it carries
                // *which* project failed, which is data, not framing.
                let state = resolve_default_mode(Path::new(&p.original_path)).map_err(|e| {
                    ErrorDto::with_params(
                        codes::PERMISSION_READ_SETTINGS_FOR_PROJECT,
                        serde_json::json!({
                            "project_path": p.original_path,
                            "detail": e.to_string(),
                        }),
                        format!("read permission settings for {}: {e}", p.original_path),
                    )
                })?;
                let active = eval::active_grant(&file, &p.original_path, now);
                Ok(project_permission_dto(
                    p.original_path.clone(),
                    &state,
                    active,
                    installed,
                ))
            })
            .collect::<Result<Vec<_>, _>>()
    })
    .await
    .map_err(ErrorDto::task_join)?
}

/// One project's permission state. The single-project sibling of
/// [`permission_list`] — used by the ProjectDetail panel so opening a
/// project doesn't trigger a full project-tree scan.
#[tauri::command]
pub async fn permission_get(project_path: String) -> Result<ProjectPermissionDto, ErrorDto> {
    tauri::async_runtime::spawn_blocking(move || {
        let file = load_grants()?;
        let state = resolve_default_mode(Path::new(&project_path)).map_err(ErrorDto::from)?;
        let active = eval::active_grant(&file, &project_path, Utc::now());
        Ok(project_permission_dto(
            project_path.clone(),
            &state,
            active,
            hook_installed(),
        ))
    })
    .await
    .map_err(ErrorDto::task_join)?
}

/// Grant a project auto-approval for `duration_secs`, recording a
/// grant the hook answers for and the orchestrator expires.
///
/// `duration_secs`: `None` → sticky grant (no auto-expiry);
/// `Some(secs)` → time-boxed, must lie in the `validate_duration`
/// range. Wire-contract note: a missing `durationSecs` JSON key
/// deserializes the same as an explicit `null` (both → sticky). That
/// is acceptable under the IPC trust model in
/// `.claude/rules/architecture.md`: the renderer is our own code and
/// the TS API types the field as `number | null`.
///
/// Re-granting a project that already has a grant replaces its
/// deadline; the hook reads the file, so the change is live at the
/// next tool call.
#[tauri::command]
pub async fn permission_grant(
    project_path: String,
    duration_secs: Option<u64>,
) -> Result<ProjectPermissionDto, ErrorDto> {
    let now = Utc::now();
    let expires_at = resolve_expires_at(duration_secs, now)?;

    tauri::async_runtime::spawn_blocking(move || {
        validate_project_path(&project_path)
            .map_err(|detail| ErrorDto::detail(codes::PERMISSION_INVALID_PROJECT_PATH, detail))?;
        // Hold the grants-file lock across load → mutate → save →
        // reconcile so an orchestrator tick can't save an older
        // snapshot over this grant or read "no grants" and take the
        // hook out. See `permission_orchestrator::grants_file_guard`.
        let _guard = grants_file_guard();
        let mut file = load_grants()?;
        let previous = file.upsert(Grant {
            project_path: project_path.clone(),
            granted_at: now,
            expires_at,
        });

        // A grant with no hook behind it is inert. Roll it back rather
        // than report an active grant that answers nothing — and on a
        // re-grant, roll back to the grant that was there, not to none.
        let path = project_path.clone();
        commit_or_rollback(
            &mut file,
            move |f| match previous {
                Some(prior) => {
                    f.upsert(prior);
                }
                None => {
                    f.remove(&path);
                }
            },
            save_and_reconcile,
            save_rollback_to_disk,
        )?;

        current_dto(&project_path)
    })
    .await
    .map_err(ErrorDto::task_join)?
}

/// Revoke a project's grant immediately. Errors if the project has no
/// grant. The hook entry leaves with the last grant.
#[tauri::command]
pub async fn permission_revert(project_path: String) -> Result<ProjectPermissionDto, ErrorDto> {
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = grants_file_guard();
        let mut file = load_grants()?;
        if file.remove(&project_path).is_none() {
            return Err(no_active_grant(&project_path));
        }
        // An uninstall that fails leaves an entry the hook itself makes
        // inert (no grant in the file → silence), so it is a warning
        // here rather than a failed revert.
        permission_store::save(&file).map_err(ErrorDto::from)?;
        if let Err(e) = reconcile_hook(&file, Utc::now()) {
            tracing::warn!(error = %e, "permission_revert: hook reconcile failed");
        }
        current_dto(&project_path)
    })
    .await
    .map_err(ErrorDto::task_join)?
}

/// Update a grant's deadline. `Some(secs)` pushes the deadline out
/// to `secs` from now (time-boxed); `None` converts the grant to
/// **sticky** — no auto-expiry. Errors if the project has no grant.
#[tauri::command]
pub async fn permission_extend(
    project_path: String,
    duration_secs: Option<u64>,
) -> Result<ProjectPermissionDto, ErrorDto> {
    let now = Utc::now();
    let expires_at = resolve_expires_at(duration_secs, now)?;
    tauri::async_runtime::spawn_blocking(move || {
        let _guard = grants_file_guard();
        let mut file = load_grants()?;
        let grant = file
            .grants
            .iter_mut()
            .find(|g| g.project_path == project_path)
            .ok_or_else(|| no_active_grant(&project_path))?;
        let prior_deadline = std::mem::replace(&mut grant.expires_at, expires_at);

        // A deadline the command failed to report must not stick.
        let path = project_path.clone();
        commit_or_rollback(
            &mut file,
            move |f| {
                if let Some(g) = f.grants.iter_mut().find(|g| g.project_path == path) {
                    g.expires_at = prior_deadline;
                }
            },
            save_and_reconcile,
            save_rollback_to_disk,
        )?;
        current_dto(&project_path)
    })
    .await
    .map_err(ErrorDto::task_join)?
}

/// Remove a `bypassPermissions` / `auto` value from the project's
/// `.claude/settings.local.json` — a key CC has ignored since 2.1.257,
/// which an older Claudepot's grant is the likeliest thing to have
/// left there. Refuses when the ignored value is in the committed
/// `.claude/settings.json`: that file is the repository's, and the
/// pane says to edit it by hand instead.
#[tauri::command]
pub async fn permission_clear_ignored(
    project_path: String,
) -> Result<ProjectPermissionDto, ErrorDto> {
    tauri::async_runtime::spawn_blocking(move || {
        validate_project_path(&project_path)
            .map_err(|detail| ErrorDto::detail(codes::PERMISSION_INVALID_PROJECT_PATH, detail))?;
        let root = Path::new(&project_path);
        let state = resolve_default_mode(root).map_err(ErrorDto::from)?;
        let layer = clearable_ignored_layer(&state).ok_or_else(|| {
            ErrorDto::with_params(
                codes::PERMISSION_NO_IGNORED_VALUE,
                serde_json::json!({ "project_path": project_path }),
                format!("no ignored project-scope value Claudepot can remove for {project_path}"),
            )
        })?;
        clear_default_mode(layer, root).map_err(ErrorDto::from)?;
        current_dto(&project_path)
    })
    .await
    .map_err(ErrorDto::task_join)?
}

/// Pure: the layer `permission_clear_ignored` may write, if the state
/// carries an ignored value in one Claudepot is allowed to touch.
fn clearable_ignored_layer(state: &PermissionState) -> Option<SettingsLayer> {
    match state.ignored.as_ref().map(|v| v.layer) {
        Some(SettingsLayer::LocalProject) => Some(SettingsLayer::LocalProject),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use claudepot_core::permission::settings::{IgnoredValue, PermissionDecisionSource};
    use claudepot_core::permission::PermissionMode;

    fn state(ignored: Option<IgnoredValue>) -> PermissionState {
        PermissionState {
            effective: PermissionMode::Default,
            decided_by: if ignored.is_some() {
                PermissionDecisionSource::ProjectScopeIgnored
            } else {
                PermissionDecisionSource::Default
            },
            user_value: None,
            project_value: None,
            local_project_value: None,
            ignored,
        }
    }

    #[test]
    fn only_the_local_file_is_clearable() {
        let local = state(Some(IgnoredValue {
            layer: SettingsLayer::LocalProject,
            mode: PermissionMode::BypassPermissions,
        }));
        assert_eq!(
            clearable_ignored_layer(&local),
            Some(SettingsLayer::LocalProject)
        );
        // The committed file is the repository's; Claudepot never
        // writes it, so the pane must not offer to.
        let committed = state(Some(IgnoredValue {
            layer: SettingsLayer::Project,
            mode: PermissionMode::Auto,
        }));
        assert_eq!(clearable_ignored_layer(&committed), None);
        assert_eq!(clearable_ignored_layer(&state(None)), None);
    }

    fn grant_at(path: &str, expires: Option<chrono::DateTime<Utc>>) -> Grant {
        Grant {
            project_path: path.into(),
            granted_at: Utc::now(),
            expires_at: expires,
        }
    }

    fn fail(_: &claudepot_core::permission::grants::GrantsFile) -> Result<(), ErrorDto> {
        Err(ErrorDto::detail(
            codes::PERMISSION_HOOK_INSTALL_FAILED,
            "simulated hook install failure",
        ))
    }

    /// Records what the rollback would have persisted, so the tests
    /// assert on it without touching a real data dir.
    fn recorder() -> (
        std::rc::Rc<std::cell::RefCell<Option<claudepot_core::permission::grants::GrantsFile>>>,
        impl FnOnce(&claudepot_core::permission::grants::GrantsFile),
    ) {
        let saved = std::rc::Rc::new(std::cell::RefCell::new(None));
        let sink = saved.clone();
        (
            saved,
            move |f: &claudepot_core::permission::grants::GrantsFile| {
                *sink.borrow_mut() = Some(f.clone());
            },
        )
    }

    #[test]
    fn a_failed_regrant_restores_the_grant_that_was_there() {
        // The rollback used to `remove` — correct for a first grant,
        // and a silent revocation for a re-grant whose hook install
        // failed. The prior grant must come back exactly.
        let mut file = claudepot_core::permission::grants::GrantsFile::default();
        let prior = grant_at("/p/a", None);
        file.upsert(prior.clone());
        let previous = file.upsert(grant_at("/p/a", Some(Utc::now() + Duration::hours(1))));
        assert_eq!(previous.as_ref(), Some(&prior));

        let (saved, record) = recorder();
        let err = commit_or_rollback(
            &mut file,
            move |f| match previous {
                Some(p) => {
                    f.upsert(p);
                }
                None => {
                    f.remove("/p/a");
                }
            },
            fail,
            record,
        )
        .unwrap_err();
        assert_eq!(err.code, codes::PERMISSION_HOOK_INSTALL_FAILED);
        assert_eq!(
            file.find("/p/a"),
            Some(&prior),
            "the earlier grant is back, deadline and all"
        );
        assert_eq!(saved.borrow().as_ref().unwrap().find("/p/a"), Some(&prior));
    }

    #[test]
    fn a_failed_first_grant_leaves_no_grant_behind() {
        let mut file = claudepot_core::permission::grants::GrantsFile::default();
        let previous = file.upsert(grant_at("/p/a", None));
        assert!(previous.is_none());
        let (saved, record) = recorder();
        let _ = commit_or_rollback(
            &mut file,
            move |f| match previous {
                Some(p) => {
                    f.upsert(p);
                }
                None => {
                    f.remove("/p/a");
                }
            },
            fail,
            record,
        );
        assert!(file.find("/p/a").is_none());
        assert!(saved.borrow().as_ref().unwrap().grants.is_empty());
    }

    #[test]
    fn a_failed_extend_keeps_the_old_deadline() {
        // `extend` used to save the new deadline and then fail on the
        // hook reconcile, so a command that reported failure had still
        // moved the deadline.
        let mut file = claudepot_core::permission::grants::GrantsFile::default();
        let old_deadline = Some(Utc::now() + Duration::minutes(30));
        file.upsert(grant_at("/p/a", old_deadline));
        let g = file
            .grants
            .iter_mut()
            .find(|g| g.project_path == "/p/a")
            .unwrap();
        let prior = g.expires_at.take();
        assert_eq!(prior, old_deadline);

        let (saved, record) = recorder();
        let _ = commit_or_rollback(
            &mut file,
            move |f| {
                if let Some(g) = f.grants.iter_mut().find(|g| g.project_path == "/p/a") {
                    g.expires_at = prior;
                }
            },
            fail,
            record,
        );
        assert_eq!(file.find("/p/a").unwrap().expires_at, old_deadline);
        // And the rollback is what gets persisted, so the hook reads
        // the old deadline too.
        assert_eq!(
            saved
                .borrow()
                .as_ref()
                .unwrap()
                .find("/p/a")
                .unwrap()
                .expires_at,
            old_deadline
        );
    }

    #[test]
    fn a_successful_persist_keeps_the_change_and_saves_nothing_extra() {
        let mut file = claudepot_core::permission::grants::GrantsFile::default();
        file.upsert(grant_at("/p/a", None));
        let (saved, record) = recorder();
        let got = commit_or_rollback(
            &mut file,
            |f| f.grants.clear(),
            |_| Ok::<_, ErrorDto>(7),
            record,
        )
        .unwrap();
        assert_eq!(got, 7);
        assert!(file.find("/p/a").is_some(), "undo must not run on success");
        assert!(saved.borrow().is_none(), "no rollback save on success");
    }

    #[test]
    fn durations_are_bounded_and_none_is_sticky() {
        let now = Utc::now();
        assert_eq!(resolve_expires_at(None, now).unwrap(), None);
        assert!(resolve_expires_at(Some(MIN_DURATION_SECS - 1), now).is_err());
        assert!(resolve_expires_at(Some(MAX_DURATION_SECS + 1), now).is_err());
        assert_eq!(
            resolve_expires_at(Some(MIN_DURATION_SECS), now).unwrap(),
            Some(now + Duration::seconds(MIN_DURATION_SECS as i64))
        );
    }
}
