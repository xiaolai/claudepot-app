//! Desktop session swap — quit, snapshot, restore, relaunch.

use super::DesktopPlatform;
use crate::error::DesktopSwapError;
use crate::paths;
use std::path::Path;
use uuid::Uuid;

/// Snapshot the current Desktop session items into the profile dir for `account_id`.
///
/// Audit M6: if an item existed in a previous snapshot but is absent
/// from the current `data_dir`, we MUST delete it from the profile so
/// it doesn't resurrect on the next `restore`. Without this, clearing
/// cookies / signing out / etc. in Desktop left the stale data in the
/// per-account profile dir, and swap-back reintroduced it.
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
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(&src, &dst)?;
        } else {
            // Missing in current session — purge any prior snapshot of
            // this item so the profile matches live state. Silent on
            // failure (permission / IO): the stale record is a bug but
            // purging is best-effort; restore()'s phase 3 only copies
            // items that exist in the profile so a leftover entry does
            // less damage than a failing snapshot.
            if dst.is_dir() {
                let _ = std::fs::remove_dir_all(&dst);
            } else if dst.is_file() {
                let _ = std::fs::remove_file(&dst);
            }
        }
    }

    Ok(())
}

/// Restore a profile into the Desktop data dir with staging + rollback.
///
/// Strategy:
///   * Phase 1 — copy the profile into a stage dir adjacent to
///     `data_dir`. Co-locating the stage on the same filesystem lets
///     Phase 3 use `rename` (atomic) instead of `copy` (non-atomic).
///   * Phase 2 — move current `data_dir` contents to a holding dir,
///     also adjacent. On any failure the partial moves are rolled back
///     from holding → data_dir.
///   * Phase 3 — `rename` items from stage → data_dir. If the target
///     and stage live on the same filesystem this is atomic. On
///     failure we clean up partially-restored items and roll back
///     from holding.
pub fn restore(
    data_dir: &Path,
    account_id: Uuid,
    session_items: &[&str],
) -> Result<(), DesktopSwapError> {
    let profile_dir = paths::desktop_profile_dir(account_id);
    if !profile_dir.exists() {
        return Err(DesktopSwapError::NoStoredProfile(account_id));
    }

    // Stage and holding dirs go in `data_dir`'s parent (or in
    // `data_dir` itself if no parent is available) so they end up on
    // the same filesystem as the live data — required for `rename` to
    // be atomic in Phase 3. `tempfile` ignores the prefix path and
    // appends a random suffix, so the dirs remain unique under
    // concurrent restores.
    let adjacent_root = data_dir.parent().unwrap_or(data_dir);
    std::fs::create_dir_all(adjacent_root)?;

    // Phase 1: stage items to a temp dir adjacent to data_dir.
    let stage_dir = tempfile::Builder::new()
        .prefix("claudepot-desktop-stage-")
        .tempdir_in(adjacent_root)?;

    for item in session_items {
        let src = profile_dir.join(item);
        let dst = stage_dir.path().join(item);
        if src.is_dir() {
            copy_dir_recursive(&src, &dst)?;
        } else if src.is_file() {
            // Nested items like "Network/Cookies" need the parent staged.
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::copy(&src, &dst)?;
        }
    }

    // Phase 2: move current items to a holding dir adjacent to data_dir.
    let holding_dir = tempfile::Builder::new()
        .prefix("claudepot-desktop-old-")
        .tempdir_in(adjacent_root)?;

    let mut moved: Vec<String> = Vec::new();
    for item in session_items {
        let src = data_dir.join(item);
        let dst = holding_dir.path().join(item);
        if src.exists() {
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let copy_result = if src.is_dir() {
                copy_dir_recursive(&src, &dst).and_then(|_| std::fs::remove_dir_all(&src))
            } else {
                std::fs::copy(&src, &dst)
                    .map(|_| ())
                    .and_then(|_| std::fs::remove_file(&src))
            };
            if let Err(e) = copy_result {
                // Phase-2 failure: rollback already-moved items
                rollback(data_dir, holding_dir.path(), &moved);
                return Err(DesktopSwapError::FileCopyFailed(format!(
                    "backup {item} failed: {e}"
                )));
            }
            moved.push(item.to_string());
        }
    }

    // Phase 3: move staged items into data dir.
    //
    // Prefer `rename` for atomicity. Stage and data_dir share a parent
    // (Phase 1 set this up), so on a sane filesystem `rename` is a
    // single inode-table update — either the new item is fully visible
    // or it isn't. Fall back to copy (non-atomic) only if rename
    // fails, which on Unix is typically `EXDEV` (cross-device link),
    // and on Windows can be `ERROR_NOT_SAME_DEVICE` or
    // `ERROR_ACCESS_DENIED` if the destination already exists.
    let mut restored: Vec<String> = Vec::new();
    for item in session_items {
        let src = stage_dir.path().join(item);
        let dst = data_dir.join(item);
        if src.exists() {
            if let Some(parent) = dst.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            // `rename` cannot replace an existing dir on Unix and may
            // refuse on Windows. After Phase 2 the live items have
            // been moved to holding, so `dst` should be absent — but
            // be defensive in case a sibling item created the parent
            // unexpectedly.
            if dst.exists() {
                if dst.is_dir() {
                    let _ = std::fs::remove_dir_all(&dst);
                } else {
                    let _ = std::fs::remove_file(&dst);
                }
            }
            let result = match std::fs::rename(&src, &dst) {
                Ok(()) => Ok(()),
                Err(_) => {
                    // Cross-filesystem fallback: copy then remove src.
                    if src.is_dir() {
                        copy_dir_recursive(&src, &dst)
                            .and_then(|_| std::fs::remove_dir_all(&src))
                    } else {
                        std::fs::copy(&src, &dst)
                            .map(|_| ())
                            .and_then(|_| std::fs::remove_file(&src))
                    }
                }
            };
            if let Err(e) = result {
                // Phase-3 failure: clean up partially-restored targets, then rollback
                for r in &restored {
                    let p = data_dir.join(r);
                    if p.is_dir() {
                        let _ = std::fs::remove_dir_all(&p);
                    } else if p.exists() {
                        let _ = std::fs::remove_file(&p);
                    }
                }
                rollback(data_dir, holding_dir.path(), &moved);
                return Err(DesktopSwapError::FileCopyFailed(format!(
                    "restore {item} failed: {e}"
                )));
            }
            restored.push(item.to_string());
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
    crate::fs_utils::copy_dir_recursive(src, dst)
}

/// Full Desktop switch: quit -> snapshot outgoing -> restore target -> relaunch.
/// Updates `has_desktop_profile` and `active_desktop` in the store.
pub async fn switch(
    platform: &dyn DesktopPlatform,
    store: &crate::account::AccountStore,
    outgoing_id: Option<Uuid>,
    target_id: Uuid,
    no_launch: bool,
) -> Result<(), DesktopSwapError> {
    let data_dir = platform.data_dir().ok_or(DesktopSwapError::NotInstalled)?;

    if !data_dir.exists() {
        return Err(DesktopSwapError::NotInstalled);
    }

    let items = platform.session_items();

    // Windows DPAPI precheck (Phase 6 + Tier 1): if the stored
    // profile's ciphertext was encrypted under a different DPAPI
    // master key than the one this Windows session holds, Chromium
    // will reject the restored cookies/tokens on next launch.
    // Surface this BEFORE quitting Desktop so the user isn't left
    // with a dead session. macOS is always Ok — the probe is a
    // no-op there.
    match crate::services::desktop_service::check_profile_dpapi_valid(platform, target_id).await {
        Ok(true) => {}
        Ok(false) => return Err(DesktopSwapError::DpapiInvalidated),
        Err(e) => {
            tracing::warn!(
                "DPAPI precheck returned error — proceeding optimistically: {e}"
            );
        }
    }

    // Quit Desktop if running
    if platform.is_running().await {
        tracing::info!("quitting Claude Desktop...");
        platform.quit().await?;
    }

    // Snapshot outgoing
    if let Some(out_id) = outgoing_id {
        tracing::info!("saving profile for outgoing account...");
        snapshot(&data_dir, out_id, items)?;
        store
            .update_desktop_profile_flag(out_id, true)
            .map_err(|e| {
                DesktopSwapError::Io(std::io::Error::other(format!("db update failed: {e}")))
            })?;
    }

    // Restore target
    tracing::info!("restoring profile for target account...");
    restore(&data_dir, target_id, items)?;

    // Update active pointer in store AFTER disk restore so metadata
    // matches on-disk reality. If the DB write fails here, disk is
    // already at target but the store still says outgoing — drift.
    // Audit M7: attempt to roll the DISK back to the outgoing snapshot
    // we just took, so the pair (disk, pointer) stays consistent
    // regardless of outcome. If the disk rollback also fails, surface
    // a combined error that names the drift so the user can reconcile.
    if let Err(db_err) = store.set_active_desktop(target_id) {
        tracing::error!(
            target = %target_id,
            "DB set_active_desktop failed after disk restore; attempting disk rollback: {db_err}"
        );
        if let Some(out_id) = outgoing_id {
            match restore(&data_dir, out_id, items) {
                Ok(()) => {
                    tracing::warn!(
                        "disk rolled back to outgoing account; state is consistent with DB"
                    );
                    return Err(DesktopSwapError::Io(std::io::Error::other(format!(
                        "db update failed (disk rolled back): {db_err}"
                    ))));
                }
                Err(rb_err) => {
                    tracing::error!(
                        "disk rollback also failed — manual reconciliation needed: {rb_err}"
                    );
                    return Err(DesktopSwapError::Io(std::io::Error::other(format!(
                        "db update failed: {db_err}; disk rollback failed: {rb_err}; \
                         Desktop is at {target_id} but DB still points to previous account — \
                         run switch again or reconcile manually"
                    ))));
                }
            }
        } else {
            // No outgoing snapshot to roll back to. Drift is real:
            // disk is at target, DB pointer is still whatever it was.
            return Err(DesktopSwapError::Io(std::io::Error::other(format!(
                "db update failed (no outgoing snapshot to roll back to): {db_err}"
            ))));
        }
    }

    // Relaunch
    if !no_launch {
        tracing::info!("launching Claude Desktop...");
        platform.launch().await?;
    }

    Ok(())
}

#[cfg(test)]
#[path = "swap_tests.rs"]
mod tests;
