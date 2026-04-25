//! Tauri commands for the Protected Paths pane.
//!
//! See `.claude/rules/architecture.md`: this file is a thin DTO shim.
//! All business logic lives in `claudepot_core::protected_paths`.
//!
//! Bodies run inside `tokio::task::spawn_blocking` so the underlying
//! synchronous JSON file I/O cannot stall Tauri's IPC worker pool
//! (audit B8 commands_protected.rs:12).

use crate::dto::ProtectedPathDto;
use claudepot_core::paths;

fn join_blocking_err(e: tokio::task::JoinError) -> String {
    format!("blocking task failed: {e}")
}

/// Materialized list (defaults minus removed_defaults, plus user
/// entries). UI renders this directly.
#[tauri::command]
pub async fn protected_paths_list() -> Result<Vec<ProtectedPathDto>, String> {
    tokio::task::spawn_blocking(|| {
        let dir = paths::claudepot_data_dir();
        let list = claudepot_core::protected_paths::list(&dir)
            .map_err(|e| format!("protected paths list failed: {e}"))?;
        Ok::<_, String>(list.iter().map(ProtectedPathDto::from).collect())
    })
    .await
    .map_err(join_blocking_err)?
}

/// Add a path. Returns the materialized entry (so the UI knows which
/// badge — default-revived vs new user — to render). Validation is in
/// core; map errors to user-facing strings here.
#[tauri::command]
pub async fn protected_paths_add(path: String) -> Result<ProtectedPathDto, String> {
    tokio::task::spawn_blocking(move || {
        let dir = paths::claudepot_data_dir();
        let added = claudepot_core::protected_paths::add(&dir, &path)
            .map_err(|e| format!("{e}"))?;
        Ok::<_, String>(ProtectedPathDto::from(&added))
    })
    .await
    .map_err(join_blocking_err)?
}

/// Remove a path. Defaults are tombstoned; user entries are dropped.
#[tauri::command]
pub async fn protected_paths_remove(path: String) -> Result<(), String> {
    tokio::task::spawn_blocking(move || {
        let dir = paths::claudepot_data_dir();
        claudepot_core::protected_paths::remove(&dir, &path).map_err(|e| format!("{e}"))
    })
    .await
    .map_err(join_blocking_err)?
}

/// Restore the implicit defaults — clears both `removed_defaults` and
/// `user`. Returns the resulting materialized list so the UI can
/// refresh in one round-trip.
#[tauri::command]
pub async fn protected_paths_reset() -> Result<Vec<ProtectedPathDto>, String> {
    tokio::task::spawn_blocking(|| {
        let dir = paths::claudepot_data_dir();
        claudepot_core::protected_paths::reset(&dir)
            .map_err(|e| format!("protected paths reset failed: {e}"))?;
        let list = claudepot_core::protected_paths::list(&dir)
            .map_err(|e| format!("protected paths list failed: {e}"))?;
        Ok::<_, String>(list.iter().map(ProtectedPathDto::from).collect())
    })
    .await
    .map_err(join_blocking_err)?
}
