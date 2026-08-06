//! Tauri commands for the Protected Paths pane.
//!
//! See `.claude/rules/architecture.md`: this file is a thin DTO shim.
//! All business logic lives in `claudepot_core::protected_paths`.
//!
//! Bodies run inside `tokio::task::spawn_blocking` so the underlying
//! synchronous JSON file I/O cannot stall Tauri's IPC worker pool
//! (audit B8 commands_protected.rs:12).

use crate::dto::ProtectedPathDto;
use crate::dto_error::ErrorDto;
use claudepot_core::paths;

/// Materialized list (defaults minus removed_defaults, plus user
/// entries). UI renders this directly.
#[tauri::command]
pub async fn protected_paths_list() -> Result<Vec<ProtectedPathDto>, ErrorDto> {
    tokio::task::spawn_blocking(|| {
        let dir = paths::claudepot_data_dir();
        // The `protected paths list failed: ` prefix moves to the UI,
        // which knows what it was loading; core's English survives as
        // `ErrorDto.message`.
        let list = claudepot_core::protected_paths::list(&dir).map_err(ErrorDto::from)?;
        Ok::<_, ErrorDto>(list.iter().map(ProtectedPathDto::from).collect())
    })
    .await
    .map_err(ErrorDto::task_join)?
}

/// Add a path. Returns the materialized entry (so the UI knows which
/// badge — default-revived vs new user — to render). Validation is in
/// core; the codes it raises (`protected_paths.not_absolute`,
/// `.duplicate`) are what the pane's inline add-form error renders.
#[tauri::command]
pub async fn protected_paths_add(path: String) -> Result<ProtectedPathDto, ErrorDto> {
    tokio::task::spawn_blocking(move || {
        let dir = paths::claudepot_data_dir();
        let added = claudepot_core::protected_paths::add(&dir, &path).map_err(ErrorDto::from)?;
        Ok::<_, ErrorDto>(ProtectedPathDto::from(&added))
    })
    .await
    .map_err(ErrorDto::task_join)?
}

/// Remove a path. Defaults are tombstoned; user entries are dropped.
#[tauri::command]
pub async fn protected_paths_remove(path: String) -> Result<(), ErrorDto> {
    tokio::task::spawn_blocking(move || {
        let dir = paths::claudepot_data_dir();
        claudepot_core::protected_paths::remove(&dir, &path).map_err(ErrorDto::from)
    })
    .await
    .map_err(ErrorDto::task_join)?
}

/// Restore the implicit defaults — clears both `removed_defaults` and
/// `user`. Returns the resulting materialized list so the UI can
/// refresh in one round-trip.
#[tauri::command]
pub async fn protected_paths_reset() -> Result<Vec<ProtectedPathDto>, ErrorDto> {
    tokio::task::spawn_blocking(|| {
        let dir = paths::claudepot_data_dir();
        claudepot_core::protected_paths::reset(&dir).map_err(ErrorDto::from)?;
        let list = claudepot_core::protected_paths::list(&dir).map_err(ErrorDto::from)?;
        Ok::<_, ErrorDto>(list.iter().map(ProtectedPathDto::from).collect())
    })
    .await
    .map_err(ErrorDto::task_join)?
}
