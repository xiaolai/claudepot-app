//! Desktop session swap — quit, snapshot, restore, relaunch.

use crate::error::DesktopSwapError;
use crate::paths;
use super::DesktopPlatform;
use uuid::Uuid;
use std::path::Path;

/// Snapshot the current Desktop session items into the profile dir for `account_id`.
pub fn snapshot(
    data_dir: &Path,
    account_id: Uuid,
    session_items: &[&str],
) -> Result<(), DesktopSwapError> {
    let profile_dir = paths::desktop_profile_dir(account_id);
    std::fs::create_dir_all(&profile_dir)?;

    for item in session_items {
        let src = data_dir.join(item);
        let dst = profile_dir.join(item);

        if src.is_dir() {
            if dst.exists() {
                std::fs::remove_dir_all(&dst)?;
            }
            copy_dir_recursive(&src, &dst)?;
        } else if src.is_file() {
            std::fs::copy(&src, &dst)?;
        }
        // Missing items are OK — not all items exist on all platforms
    }

    Ok(())
}

/// Restore a profile into the Desktop data dir with staging + rollback.
pub fn restore(
    data_dir: &Path,
    account_id: Uuid,
    session_items: &[&str],
) -> Result<(), DesktopSwapError> {
    let profile_dir = paths::desktop_profile_dir(account_id);
    if !profile_dir.exists() {
        return Err(DesktopSwapError::NoStoredProfile(account_id));
    }

    // Phase 1: stage items to a temp dir
    let stage_dir = tempfile::Builder::new()
        .prefix("claudepot-desktop-stage-")
        .tempdir()?;

    for item in session_items {
        let src = profile_dir.join(item);
        let dst = stage_dir.path().join(item);
        if src.is_dir() {
            copy_dir_recursive(&src, &dst)?;
        } else if src.is_file() {
            std::fs::copy(&src, &dst)?;
        }
    }

    // Phase 2: move current items to a holding dir
    let holding_dir = tempfile::Builder::new()
        .prefix("claudepot-desktop-old-")
        .tempdir()?;

    let mut moved: Vec<String> = Vec::new();
    for item in session_items {
        let src = data_dir.join(item);
        let dst = holding_dir.path().join(item);
        if src.exists() {
            if src.is_dir() {
                copy_dir_recursive(&src, &dst)?;
                std::fs::remove_dir_all(&src)?;
            } else {
                std::fs::copy(&src, &dst)?;
                std::fs::remove_file(&src)?;
            }
            moved.push(item.to_string());
        }
    }

    // Phase 3: move staged items into data dir
    for item in session_items {
        let src = stage_dir.path().join(item);
        let dst = data_dir.join(item);
        if src.exists() {
            if src.is_dir() {
                if let Err(e) = copy_dir_recursive(&src, &dst) {
                    // Rollback: restore from holding
                    rollback(data_dir, holding_dir.path(), &moved);
                    return Err(DesktopSwapError::FileCopyFailed(
                        format!("restore {item} failed: {e}"),
                    ));
                }
            } else if let Err(e) = std::fs::copy(&src, &dst) {
                rollback(data_dir, holding_dir.path(), &moved);
                return Err(DesktopSwapError::FileCopyFailed(
                    format!("restore {item} failed: {e}"),
                ));
            }
        }
    }

    Ok(())
}

fn rollback(data_dir: &Path, holding_dir: &Path, items: &[String]) {
    for item in items {
        let src = holding_dir.join(item);
        let dst = data_dir.join(item);
        if src.is_dir() {
            let _ = copy_dir_recursive(&src, &dst);
        } else if src.is_file() {
            let _ = std::fs::copy(&src, &dst);
        }
    }
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            std::fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

/// Full Desktop switch: quit → snapshot outgoing → restore target → relaunch.
/// Updates `has_desktop_profile` and `active_desktop` in the store.
pub async fn switch(
    platform: &dyn DesktopPlatform,
    store: &crate::account::AccountStore,
    outgoing_id: Option<Uuid>,
    target_id: Uuid,
    no_launch: bool,
) -> Result<(), DesktopSwapError> {
    let data_dir = platform.data_dir()
        .ok_or(DesktopSwapError::NotInstalled)?;

    if !data_dir.exists() {
        return Err(DesktopSwapError::NotInstalled);
    }

    let items = platform.session_items();

    // Quit Desktop if running
    if platform.is_running().await {
        tracing::info!("quitting Claude Desktop...");
        platform.quit().await?;
    }

    // Snapshot outgoing
    if let Some(out_id) = outgoing_id {
        tracing::info!("saving profile for outgoing account...");
        snapshot(&data_dir, out_id, items)?;
        let _ = store.update_desktop_profile_flag(out_id, true);
    }

    // Restore target
    tracing::info!("restoring profile for target account...");
    restore(&data_dir, target_id, items)?;

    // Update active pointer in store (before relaunch so state is consistent)
    let _ = store.set_active_desktop(target_id);

    // Relaunch
    if !no_launch {
        tracing::info!("launching Claude Desktop...");
        platform.launch().await?;
    }

    Ok(())
}
