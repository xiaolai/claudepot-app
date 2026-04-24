//! Tauri commands for session move / orphan adoption / discard.
//!
//! All business logic lives in `claudepot_core::session_move`; this
//! module is a DTO shim + validation at the edge.

use claudepot_core::paths;
use uuid::Uuid;

#[tauri::command]
pub async fn session_list_orphans() -> Result<Vec<crate::dto::OrphanedProjectDto>, String> {
    let cfg = paths::claude_config_dir();
    let orphans = claudepot_core::session_move::detect_orphaned_projects(&cfg)
        .map_err(|e| format!("orphan scan failed: {e}"))?;
    Ok(orphans
        .iter()
        .map(crate::dto::OrphanedProjectDto::from)
        .collect())
}

/// CC stores `.claude.json` at `$HOME/.claude.json` — a sibling of
/// `~/.claude/`. Central accessor so the Tauri layer agrees with CLI.
fn claude_json_path() -> Option<std::path::PathBuf> {
    dirs::home_dir().map(|h| h.join(".claude.json"))
}

#[tauri::command]
pub async fn session_move(
    session_id: String,
    from_cwd: String,
    to_cwd: String,
    force_live: bool,
    force_conflict: bool,
    cleanup_source: bool,
) -> Result<crate::dto::MoveSessionReportDto, String> {
    let sid = Uuid::parse_str(&session_id)
        .map_err(|e| format!("invalid session id: {e}"))?;
    let cfg = paths::claude_config_dir();
    let opts = claudepot_core::session_move::MoveSessionOpts {
        force_live_session: force_live,
        force_sync_conflict: force_conflict,
        cleanup_source_if_empty: cleanup_source,
        claude_json_path: claude_json_path(),
    };
    let report = claudepot_core::session_move::move_session(
        &cfg,
        sid,
        std::path::Path::new(&from_cwd),
        std::path::Path::new(&to_cwd),
        opts,
    )
    .map_err(|e| format!("move failed: {e}"))?;
    Ok(crate::dto::MoveSessionReportDto::from(&report))
}

#[tauri::command]
pub async fn session_adopt_orphan(
    slug: String,
    target_cwd: String,
) -> Result<crate::dto::AdoptReportDto, String> {
    let cfg = paths::claude_config_dir();
    let target = std::path::Path::new(&target_cwd);
    if !target.is_dir() {
        return Err(format!("target cwd does not exist: {target_cwd}"));
    }
    let report =
        claudepot_core::session_move::adopt_orphan_project(&cfg, &slug, target, claude_json_path())
            .map_err(|e| format!("adopt failed: {e}"))?;
    Ok(crate::dto::AdoptReportDto::from(&report))
}

/// Move an orphan project slug dir to the OS Trash. The user can restore
/// it from Trash if they change their mind; the guard that the slug is
/// valid + resolves to a real dir happens in core.
#[tauri::command]
pub async fn session_discard_orphan(
    slug: String,
) -> Result<crate::dto::DiscardReportDto, String> {
    let cfg = paths::claude_config_dir();
    let report = claudepot_core::session_move::discard_orphan_project(&cfg, &slug)
        .map_err(|e| format!("discard failed: {e}"))?;
    Ok(crate::dto::DiscardReportDto::from(&report))
}
