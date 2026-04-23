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
            // Nested items like "Network/Cookies" need the parent staged.
            if let Some(parent) = dst.parent() {
                std::fs::create_dir_all(parent)?;
            }
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

    // Phase 3: move staged items into data dir
    let mut restored: Vec<String> = Vec::new();
    for item in session_items {
        let src = stage_dir.path().join(item);
        let dst = data_dir.join(item);
        if src.exists() {
            if let Some(parent) = dst.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let result = if src.is_dir() {
                copy_dir_recursive(&src, &dst)
            } else {
                std::fs::copy(&src, &dst).map(|_| ())
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
mod tests {
    use super::*;
    use crate::error::DesktopSwapError;
    use crate::testing::{make_account, setup_test_data_dir, test_store, DATA_DIR_LOCK};
    use std::fs;
    use std::sync::atomic::{AtomicBool, Ordering};

    const TEST_ITEMS: &[&str] = &["config.json", "Cookies", "Local Storage"];

    fn populate_data_dir(data_dir: &Path, items: &[&str]) {
        for item in items {
            if item.contains("Storage") {
                let d = data_dir.join(item);
                fs::create_dir_all(&d).unwrap();
                fs::write(d.join("data.dat"), "storage-data").unwrap();
            } else {
                fs::write(data_dir.join(item), format!("content-of-{item}")).unwrap();
            }
        }
    }

    // -- snapshot tests --

    #[test]
    fn test_snapshot_captures_files() {
        let _lock = crate::testing::lock_data_dir();
        let _env_dir = setup_test_data_dir();
        let data_dir = _env_dir.path().join("Claude");
        fs::create_dir_all(&data_dir).unwrap();
        populate_data_dir(&data_dir, TEST_ITEMS);

        let account_id = Uuid::new_v4();
        snapshot(&data_dir, account_id, TEST_ITEMS).unwrap();

        let profile = crate::paths::desktop_profile_dir(account_id);
        assert_eq!(
            fs::read_to_string(profile.join("config.json")).unwrap(),
            "content-of-config.json"
        );
        assert_eq!(
            fs::read_to_string(profile.join("Cookies")).unwrap(),
            "content-of-Cookies"
        );
    }

    #[test]
    fn test_snapshot_captures_directories() {
        let _lock = crate::testing::lock_data_dir();
        let _env_dir = setup_test_data_dir();
        let data_dir = _env_dir.path().join("Claude");
        fs::create_dir_all(&data_dir).unwrap();
        populate_data_dir(&data_dir, TEST_ITEMS);

        let account_id = Uuid::new_v4();
        snapshot(&data_dir, account_id, TEST_ITEMS).unwrap();

        let profile = crate::paths::desktop_profile_dir(account_id);
        let storage = profile.join("Local Storage");
        assert!(storage.is_dir());
        assert_eq!(
            fs::read_to_string(storage.join("data.dat")).unwrap(),
            "storage-data"
        );
    }

    #[test]
    fn test_snapshot_skips_missing_items() {
        let _lock = crate::testing::lock_data_dir();
        let _env_dir = setup_test_data_dir();
        let data_dir = _env_dir.path().join("Claude");
        fs::create_dir_all(&data_dir).unwrap();
        // Only create one of three items
        fs::write(data_dir.join("config.json"), "cfg").unwrap();

        let account_id = Uuid::new_v4();
        // Should NOT error even though "Cookies" and "Local Storage" are missing
        snapshot(&data_dir, account_id, TEST_ITEMS).unwrap();

        let profile = crate::paths::desktop_profile_dir(account_id);
        assert!(profile.join("config.json").exists());
        assert!(!profile.join("Cookies").exists());
    }

    #[test]
    fn test_snapshot_purges_items_absent_from_current_session() {
        // Audit M6 regression guard. If a prior snapshot captured
        // `Cookies` but the user then cleared cookies in Desktop and
        // we snapshot again, the profile dir MUST NOT retain the
        // stale cookies — otherwise a later restore resurrects them.
        let _lock = crate::testing::lock_data_dir();
        let _env_dir = setup_test_data_dir();
        let data_dir = _env_dir.path().join("Claude");
        fs::create_dir_all(&data_dir).unwrap();

        let account_id = Uuid::new_v4();
        let profile = crate::paths::desktop_profile_dir(account_id);

        // Round 1: data_dir has all three items → snapshot captures all.
        fs::write(data_dir.join("config.json"), "v1").unwrap();
        fs::write(data_dir.join("Cookies"), "yum").unwrap();
        fs::create_dir_all(data_dir.join("Local Storage")).unwrap();
        fs::write(data_dir.join("Local Storage/data.dat"), "ls").unwrap();
        snapshot(&data_dir, account_id, TEST_ITEMS).unwrap();
        assert!(profile.join("Cookies").exists());
        assert!(profile.join("Local Storage").is_dir());

        // Round 2: Cookies and Local Storage removed from data_dir.
        fs::remove_file(data_dir.join("Cookies")).unwrap();
        fs::remove_dir_all(data_dir.join("Local Storage")).unwrap();
        snapshot(&data_dir, account_id, TEST_ITEMS).unwrap();

        // Stale entries must be purged from the profile.
        assert!(
            !profile.join("Cookies").exists(),
            "stale Cookies not purged"
        );
        assert!(
            !profile.join("Local Storage").exists(),
            "stale Local Storage dir not purged"
        );
        // config.json still present in data_dir → still in profile.
        assert_eq!(fs::read_to_string(profile.join("config.json")).unwrap(), "v1");
    }

    #[test]
    fn test_snapshot_overwrites_previous() {
        let _lock = crate::testing::lock_data_dir();
        let _env_dir = setup_test_data_dir();
        let data_dir = _env_dir.path().join("Claude");
        fs::create_dir_all(&data_dir).unwrap();

        let account_id = Uuid::new_v4();

        // First snapshot
        fs::write(data_dir.join("config.json"), "v1").unwrap();
        snapshot(&data_dir, account_id, &["config.json"]).unwrap();

        // Second snapshot with different content
        fs::write(data_dir.join("config.json"), "v2").unwrap();
        snapshot(&data_dir, account_id, &["config.json"]).unwrap();

        let profile = crate::paths::desktop_profile_dir(account_id);
        assert_eq!(
            fs::read_to_string(profile.join("config.json")).unwrap(),
            "v2"
        );
    }

    // -- restore tests --

    #[test]
    fn test_restore_populates_data_dir() {
        let _lock = crate::testing::lock_data_dir();
        let _env_dir = setup_test_data_dir();
        let data_dir = _env_dir.path().join("Claude");
        fs::create_dir_all(&data_dir).unwrap();

        let account_id = Uuid::new_v4();
        // Pre-create profile
        let profile = crate::paths::desktop_profile_dir(account_id);
        fs::create_dir_all(&profile).unwrap();
        fs::write(profile.join("config.json"), "restored-config").unwrap();

        restore(&data_dir, account_id, &["config.json"]).unwrap();

        assert_eq!(
            fs::read_to_string(data_dir.join("config.json")).unwrap(),
            "restored-config"
        );
    }

    #[test]
    fn test_restore_replaces_existing() {
        let _lock = crate::testing::lock_data_dir();
        let _env_dir = setup_test_data_dir();
        let data_dir = _env_dir.path().join("Claude");
        fs::create_dir_all(&data_dir).unwrap();
        fs::write(data_dir.join("config.json"), "old-content").unwrap();

        let account_id = Uuid::new_v4();
        let profile = crate::paths::desktop_profile_dir(account_id);
        fs::create_dir_all(&profile).unwrap();
        fs::write(profile.join("config.json"), "new-content").unwrap();

        restore(&data_dir, account_id, &["config.json"]).unwrap();

        assert_eq!(
            fs::read_to_string(data_dir.join("config.json")).unwrap(),
            "new-content"
        );
    }

    #[test]
    fn test_restore_no_profile_returns_error() {
        let _lock = crate::testing::lock_data_dir();
        let _env_dir = setup_test_data_dir();

        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().join("Claude");
        fs::create_dir_all(&data_dir).unwrap();

        let account_id = Uuid::new_v4();
        let result = restore(&data_dir, account_id, &["config.json"]);
        assert!(matches!(result, Err(DesktopSwapError::NoStoredProfile(_))));
    }

    #[test]
    fn test_snapshot_then_restore_roundtrip() {
        let _lock = crate::testing::lock_data_dir();
        let _env_dir = setup_test_data_dir();
        let data_dir = _env_dir.path().join("Claude");
        fs::create_dir_all(&data_dir).unwrap();
        populate_data_dir(&data_dir, TEST_ITEMS);

        let account_id = Uuid::new_v4();

        // Snapshot
        snapshot(&data_dir, account_id, TEST_ITEMS).unwrap();

        // Clear data_dir
        for item in TEST_ITEMS {
            let p = data_dir.join(item);
            if p.is_dir() {
                fs::remove_dir_all(&p).unwrap();
            } else if p.exists() {
                fs::remove_file(&p).unwrap();
            }
        }

        // Restore
        restore(&data_dir, account_id, TEST_ITEMS).unwrap();

        // Verify
        assert_eq!(
            fs::read_to_string(data_dir.join("config.json")).unwrap(),
            "content-of-config.json"
        );
        assert!(data_dir.join("Local Storage").is_dir());
    }

    // -- MockDesktopPlatform + switch tests --

    struct MockDesktopPlatform {
        data_dir_path: Option<std::path::PathBuf>,
        items: Vec<&'static str>,
        running: AtomicBool,
        quit_called: AtomicBool,
        launch_called: AtomicBool,
        fail_quit: bool,
    }

    impl MockDesktopPlatform {
        fn new(data_dir: &Path, items: &[&'static str]) -> Self {
            Self {
                data_dir_path: Some(data_dir.to_path_buf()),
                items: items.to_vec(),
                running: AtomicBool::new(false),
                quit_called: AtomicBool::new(false),
                launch_called: AtomicBool::new(false),
                fail_quit: false,
            }
        }
    }

    #[async_trait::async_trait]
    impl DesktopPlatform for MockDesktopPlatform {
        fn data_dir(&self) -> Option<std::path::PathBuf> {
            self.data_dir_path.clone()
        }
        fn session_items(&self) -> &[&str] {
            &self.items
        }
        async fn is_running(&self) -> bool {
            self.running.load(Ordering::SeqCst)
        }
        async fn quit(&self) -> Result<(), DesktopSwapError> {
            if self.fail_quit {
                return Err(DesktopSwapError::DesktopStillRunning);
            }
            self.quit_called.store(true, Ordering::SeqCst);
            self.running.store(false, Ordering::SeqCst);
            Ok(())
        }
        async fn launch(&self) -> Result<(), DesktopSwapError> {
            self.launch_called.store(true, Ordering::SeqCst);
            self.running.store(true, Ordering::SeqCst);
            Ok(())
        }
        fn is_installed(&self) -> bool {
            // Mock: "installed" iff we were given a data_dir path.
            self.data_dir_path.is_some()
        }
        async fn safe_storage_secret(
            &self,
        ) -> Result<Vec<u8>, crate::desktop_backend::DesktopKeyError> {
            // Mock: tests that exercise identity/adopt/clear override
            // via a concrete fake — this trait impl exists only so
            // unrelated switch tests compile.
            Err(crate::desktop_backend::DesktopKeyError::Unsupported)
        }
    }

    #[tokio::test]
    async fn test_switch_full_lifecycle() {
        let _lock = crate::testing::lock_data_dir();
        let _env_dir = setup_test_data_dir();
        let data_dir = _env_dir.path().join("Claude");
        fs::create_dir_all(&data_dir).unwrap();

        let (store, _db_dir) = test_store();
        let items: &[&str] = &["config.json"];

        // Set up two accounts
        let mut out_acct = make_account("outgoing@example.com");
        let mut tgt_acct = make_account("target@example.com");
        store.insert(&out_acct).unwrap();
        store.insert(&tgt_acct).unwrap();

        // Current data dir has outgoing's data
        fs::write(data_dir.join("config.json"), "outgoing-config").unwrap();

        // Pre-create target's profile
        let tgt_profile = crate::paths::desktop_profile_dir(tgt_acct.uuid);
        fs::create_dir_all(&tgt_profile).unwrap();
        fs::write(tgt_profile.join("config.json"), "target-config").unwrap();

        let mut platform = MockDesktopPlatform::new(&data_dir, &["config.json"]);
        platform.running.store(true, Ordering::SeqCst);

        switch(&platform, &store, Some(out_acct.uuid), tgt_acct.uuid, false)
            .await
            .unwrap();

        // Verify quit was called
        assert!(platform.quit_called.load(Ordering::SeqCst));
        // Verify launch was called
        assert!(platform.launch_called.load(Ordering::SeqCst));
        // Verify data_dir now has target's config
        assert_eq!(
            fs::read_to_string(data_dir.join("config.json")).unwrap(),
            "target-config"
        );
        // Verify outgoing was snapshotted
        let out_profile = crate::paths::desktop_profile_dir(out_acct.uuid);
        assert_eq!(
            fs::read_to_string(out_profile.join("config.json")).unwrap(),
            "outgoing-config"
        );
        // Verify active desktop pointer
        assert_eq!(
            store.active_desktop_uuid().unwrap(),
            Some(tgt_acct.uuid.to_string())
        );
    }

    #[tokio::test]
    async fn test_switch_no_launch_flag() {
        let _lock = crate::testing::lock_data_dir();
        let _env_dir = setup_test_data_dir();
        let data_dir = _env_dir.path().join("Claude");
        fs::create_dir_all(&data_dir).unwrap();

        let (store, _db_dir) = test_store();
        let tgt = make_account("t@example.com");
        store.insert(&tgt).unwrap();

        let tgt_profile = crate::paths::desktop_profile_dir(tgt.uuid);
        fs::create_dir_all(&tgt_profile).unwrap();
        fs::write(tgt_profile.join("config.json"), "cfg").unwrap();

        let platform = MockDesktopPlatform::new(&data_dir, &["config.json"]);
        switch(&platform, &store, None, tgt.uuid, true)
            .await
            .unwrap();

        assert!(!platform.launch_called.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn test_switch_not_running_skips_quit() {
        let _lock = crate::testing::lock_data_dir();
        let _env_dir = setup_test_data_dir();
        let data_dir = _env_dir.path().join("Claude");
        fs::create_dir_all(&data_dir).unwrap();

        let (store, _db_dir) = test_store();
        let tgt = make_account("t@example.com");
        store.insert(&tgt).unwrap();

        let tgt_profile = crate::paths::desktop_profile_dir(tgt.uuid);
        fs::create_dir_all(&tgt_profile).unwrap();
        fs::write(tgt_profile.join("config.json"), "cfg").unwrap();

        let platform = MockDesktopPlatform::new(&data_dir, &["config.json"]);
        // running = false (default)
        switch(&platform, &store, None, tgt.uuid, false)
            .await
            .unwrap();

        assert!(!platform.quit_called.load(Ordering::SeqCst));
    }

    #[tokio::test]
    async fn test_switch_no_outgoing_skips_snapshot() {
        let _lock = crate::testing::lock_data_dir();
        let _env_dir = setup_test_data_dir();
        let data_dir = _env_dir.path().join("Claude");
        fs::create_dir_all(&data_dir).unwrap();

        let (store, _db_dir) = test_store();
        let tgt = make_account("t@example.com");
        store.insert(&tgt).unwrap();

        let tgt_profile = crate::paths::desktop_profile_dir(tgt.uuid);
        fs::create_dir_all(&tgt_profile).unwrap();
        fs::write(tgt_profile.join("config.json"), "cfg").unwrap();

        let platform = MockDesktopPlatform::new(&data_dir, &["config.json"]);

        // outgoing_id = None
        switch(&platform, &store, None, tgt.uuid, false)
            .await
            .unwrap();

        // Data dir should have target's config
        assert_eq!(
            fs::read_to_string(data_dir.join("config.json")).unwrap(),
            "cfg"
        );
    }

    #[tokio::test]
    async fn test_switch_not_installed_returns_error() {
        let (store, _db_dir) = test_store();
        let tgt = make_account("t@example.com");
        store.insert(&tgt).unwrap();

        let mut platform = MockDesktopPlatform::new(Path::new("/nonexistent"), &["config.json"]);
        platform.data_dir_path = None;

        let result = switch(&platform, &store, None, tgt.uuid, false).await;
        assert!(matches!(result, Err(DesktopSwapError::NotInstalled)));
    }

    #[tokio::test]
    async fn test_switch_quit_failure_propagates() {
        let _lock = crate::testing::lock_data_dir();
        let _env_dir = setup_test_data_dir();
        let data_dir = _env_dir.path().join("Claude");
        fs::create_dir_all(&data_dir).unwrap();

        let (store, _db_dir) = test_store();
        let tgt = make_account("t@example.com");
        store.insert(&tgt).unwrap();

        let mut platform = MockDesktopPlatform::new(&data_dir, &["config.json"]);
        platform.running.store(true, Ordering::SeqCst);
        platform.fail_quit = true;

        let result = switch(&platform, &store, None, tgt.uuid, false).await;
        assert!(matches!(result, Err(DesktopSwapError::DesktopStillRunning)));
    }

    // -------------------------------------------------------------------
    // Group 12 — Windows Desktop backend (nested session items).
    // Windows has `Network/Cookies` instead of macOS's `Cookies`. These
    // tests use MockDesktopPlatform with Windows-style items to verify
    // snapshot/restore handle parent-dir creation for nested paths.
    // -------------------------------------------------------------------

    /// The 12 Windows session items from `desktop_backend/windows.rs`.
    /// Kept in sync with `WindowsDesktop::session_items()`.
    const WINDOWS_ITEMS: &[&str] = &[
        "config.json",
        "Network/Cookies",
        "Network/Cookies-journal",
        "Network/Network Persistent State",
        "DIPS",
        "DIPS-wal",
        "Preferences",
        "ant-did",
        "git-worktrees.json",
        "Local Storage",
        "Session Storage",
        "IndexedDB",
    ];

    fn populate_windows_data_dir(data_dir: &Path) {
        fs::create_dir_all(data_dir.join("Network")).unwrap();
        for item in WINDOWS_ITEMS {
            let p = data_dir.join(item);
            if item.ends_with("Storage") || item.ends_with("IndexedDB") {
                fs::create_dir_all(&p).unwrap();
                fs::write(p.join("data.dat"), format!("dir-{item}")).unwrap();
            } else {
                if let Some(parent) = p.parent() {
                    fs::create_dir_all(parent).unwrap();
                }
                fs::write(&p, format!("file-{item}")).unwrap();
            }
        }
    }

    #[test]
    fn test_desktop_snapshot_nested_session_items() {
        let _lock = crate::testing::lock_data_dir();
        let _env_dir = setup_test_data_dir();
        let data_dir = _env_dir.path().join("Claude");
        fs::create_dir_all(&data_dir).unwrap();
        populate_windows_data_dir(&data_dir);

        let account_id = Uuid::new_v4();
        snapshot(&data_dir, account_id, WINDOWS_ITEMS).unwrap();

        let profile = crate::paths::desktop_profile_dir(account_id);
        // Parent Network/ must be created by the file-copy branch.
        assert!(profile.join("Network").is_dir(), "Network dir missing");
        assert_eq!(
            fs::read_to_string(profile.join("Network/Cookies")).unwrap(),
            "file-Network/Cookies"
        );
        assert_eq!(
            fs::read_to_string(profile.join("Network/Cookies-journal")).unwrap(),
            "file-Network/Cookies-journal"
        );
    }

    #[test]
    fn test_desktop_restore_nested_session_items() {
        let _lock = crate::testing::lock_data_dir();
        let _env_dir = setup_test_data_dir();
        let data_dir = _env_dir.path().join("Claude");
        fs::create_dir_all(&data_dir).unwrap();

        let account_id = Uuid::new_v4();
        // Pre-create a profile with nested content.
        let profile = crate::paths::desktop_profile_dir(account_id);
        fs::create_dir_all(profile.join("Network")).unwrap();
        fs::write(profile.join("Network/Cookies"), "restored-cookies").unwrap();
        fs::write(profile.join("config.json"), "restored-config").unwrap();

        restore(&data_dir, account_id, &["config.json", "Network/Cookies"]).unwrap();

        // Parent dir must be created by phase-3 create_dir_all(parent).
        assert!(data_dir.join("Network").is_dir(), "Network dir not created");
        assert_eq!(
            fs::read_to_string(data_dir.join("Network/Cookies")).unwrap(),
            "restored-cookies"
        );
        assert_eq!(
            fs::read_to_string(data_dir.join("config.json")).unwrap(),
            "restored-config"
        );
    }

    // -------------------------------------------------------------------
    // Group 3 — Desktop swap rollback (4 tests).
    // -------------------------------------------------------------------

    #[cfg(unix)]
    #[test]
    fn test_desktop_restore_phase2_failure_rollback() {
        // Phase 2 moves data_dir items to the holding dir. If a later item
        // fails to move, the already-moved items must be restored to data_dir.
        // Strategy: put `config.json` and `Cookies` directly in data_dir, and
        // `readonly_dir/file3` inside an un-removable subdir. Phase 2 moves the
        // first two successfully, then fails on file3 (remove_file inside the
        // 0o500 parent). Rollback must restore the first two.
        //
        // Skip when running as root (e.g. WSL2 default user): root ignores
        // the 0o500 perm and the test's failure-injection mechanism does
        // nothing.
        if unsafe { libc::geteuid() } == 0 {
            eprintln!("skipping: root bypasses chmod 0o500");
            return;
        }
        use std::os::unix::fs::PermissionsExt;
        let _lock = crate::testing::lock_data_dir();
        let _env_dir = setup_test_data_dir();
        let data_dir = _env_dir.path().join("Claude");
        fs::create_dir_all(&data_dir).unwrap();

        fs::write(data_dir.join("config.json"), "cfg-original").unwrap();
        fs::write(data_dir.join("Cookies"), "cookies-original").unwrap();
        let trap_dir = data_dir.join("readonly_dir");
        fs::create_dir(&trap_dir).unwrap();
        fs::write(trap_dir.join("file3"), "trapped").unwrap();

        // Profile for the account being restored (must exist).
        let account_id = Uuid::new_v4();
        let profile = crate::paths::desktop_profile_dir(account_id);
        fs::create_dir_all(&profile).unwrap();
        fs::write(profile.join("config.json"), "profile-cfg").unwrap();

        // Deny writes inside trap_dir so `remove_file` on readonly_dir/file3 fails.
        fs::set_permissions(&trap_dir, fs::Permissions::from_mode(0o500)).unwrap();

        let items: &[&str] = &["config.json", "Cookies", "readonly_dir/file3"];
        let result = restore(&data_dir, account_id, items);

        // Restore permissions BEFORE any assertion so cleanup works even on fail.
        fs::set_permissions(&trap_dir, fs::Permissions::from_mode(0o755)).unwrap();

        assert!(
            matches!(result, Err(DesktopSwapError::FileCopyFailed(_))),
            "phase 2 failure must surface as FileCopyFailed, got {:?}",
            result
        );
        // Already-moved items restored to data_dir.
        assert_eq!(
            fs::read_to_string(data_dir.join("config.json")).unwrap(),
            "cfg-original",
            "config.json must be rolled back to original"
        );
        assert_eq!(
            fs::read_to_string(data_dir.join("Cookies")).unwrap(),
            "cookies-original",
            "Cookies must be rolled back to original"
        );
    }

    #[cfg(unix)]
    #[test]
    fn test_desktop_restore_phase3_failure_cleans_partial() {
        // Phase 2 succeeds (all data_dir items moved to holding). Phase 3
        // then copies from stage into data_dir — fail on the LAST item by
        // making the data_dir itself read-only midway. The restore cleans
        // partially-restored targets and rolls back from holding.
        //
        // Strategy: stage has items [a, b, c]. data_dir initially has [a, b, c].
        // Make data_dir/c a DIRECTORY we've chmoded, so phase-3 copy into it
        // fails. Phase 2 succeeds (removes a, b, c from data_dir). Phase 3
        // starts: stage/a → data_dir/a OK; stage/b → data_dir/b OK; stage/c
        // → data_dir/c — FAILS because the parent dir changed perms.
        //
        // Simpler realization: put item `c` inside a subdir whose perms we
        // change to 0o500 AFTER phase 2 has moved things (but phase 2 runs
        // synchronously inside restore()). To fail phase 3 only, we pre-create
        // an unwritable DIRECTORY at the target path for item `c`, which
        // causes `fs::copy` to fail without disturbing phase 2.
        use std::os::unix::fs::PermissionsExt;
        let _lock = crate::testing::lock_data_dir();
        let _env_dir = setup_test_data_dir();
        let data_dir = _env_dir.path().join("Claude");
        fs::create_dir_all(&data_dir).unwrap();

        fs::write(data_dir.join("a.json"), "a-orig").unwrap();
        fs::write(data_dir.join("b.json"), "b-orig").unwrap();
        fs::write(data_dir.join("c.json"), "c-orig").unwrap();

        // Profile has content for all three items.
        let account_id = Uuid::new_v4();
        let profile = crate::paths::desktop_profile_dir(account_id);
        fs::create_dir_all(&profile).unwrap();
        fs::write(profile.join("a.json"), "a-new").unwrap();
        fs::write(profile.join("b.json"), "b-new").unwrap();
        fs::write(profile.join("c.json"), "c-new").unwrap();

        // Phase 3 copy from stage/c.json → data_dir/c.json must fail.
        // Pre-create data_dir/c.json as a DIRECTORY — phase 2 removes it (via
        // remove_file for file). Actually phase 2 uses remove_file on files;
        // if c.json is a directory, the `src.is_dir()` branch runs, calling
        // copy_dir_recursive + remove_dir_all. That succeeds too.
        //
        // Trick: put c.json inside a subdir whose DATA DIR PARENT is writable,
        // but the file CANNOT be placed at the final location. The cleanest:
        // make data_dir itself read-only AFTER phase 2 removed items but
        // BEFORE phase 3 restores. We can't inject code mid-run from a test.
        //
        // Alternative: set the stage item's PROFILE source file unreadable —
        // phase 1 staging would fail before phase 2, which isn't what we want.
        //
        // Real approach: use an item path whose PARENT doesn't exist in
        // data_dir, and phase-3's `let _ = create_dir_all(parent)` swallows
        // errors. Then `fs::copy` fails because parent doesn't exist.
        // Make the parent a FILE (not a dir) — create_dir_all will fail
        // silently and fs::copy into a nonexistent dir fails.
        fs::write(data_dir.join("blocker"), "this is a file, not a dir").unwrap();
        fs::write(profile.join("blocker/nested.json"), "x").ok();
        // Ensure the profile has a nested item where the data_dir parent is a file.
        let profile_nested = profile.join("blocker");
        let _ = fs::remove_file(&profile_nested);
        fs::create_dir_all(&profile_nested).unwrap();
        fs::write(profile_nested.join("nested.json"), "nested-new").unwrap();

        let items: &[&str] = &["a.json", "b.json", "blocker/nested.json"];
        let result = restore(&data_dir, account_id, items);

        // Cleanup-safe assertions.
        assert!(
            matches!(result, Err(DesktopSwapError::FileCopyFailed(_))),
            "phase 3 failure expected, got {:?}",
            result
        );
        // Partially-restored items cleaned up + originals rolled back.
        assert_eq!(
            fs::read_to_string(data_dir.join("a.json")).unwrap(),
            "a-orig",
            "a.json rolled back from holding"
        );
        assert_eq!(
            fs::read_to_string(data_dir.join("b.json")).unwrap(),
            "b-orig",
            "b.json rolled back from holding"
        );
    }

    #[tokio::test]
    async fn test_desktop_switch_db_profile_flag_failure_propagates() {
        // snapshot() succeeds, but the subsequent
        // store.update_desktop_profile_flag() fails because we drop the
        // accounts table. switch() must return Err rather than silently ignore.
        let _lock = crate::testing::lock_data_dir();
        let _env_dir = setup_test_data_dir();
        let data_dir = _env_dir.path().join("Claude");
        fs::create_dir_all(&data_dir).unwrap();
        fs::write(data_dir.join("config.json"), "out-cfg").unwrap();

        let (store, _db) = test_store();
        let out = make_account("out@example.com");
        let tgt = make_account("tgt@example.com");
        store.insert(&out).unwrap();
        store.insert(&tgt).unwrap();

        let tgt_profile = crate::paths::desktop_profile_dir(tgt.uuid);
        fs::create_dir_all(&tgt_profile).unwrap();
        fs::write(tgt_profile.join("config.json"), "tgt-cfg").unwrap();

        // Drop accounts table: snapshot writes files (OK), then DB update fails.
        store.corrupt_for_test();

        let platform = MockDesktopPlatform::new(&data_dir, &["config.json"]);
        let result = switch(&platform, &store, Some(out.uuid), tgt.uuid, true).await;
        assert!(
            matches!(result, Err(DesktopSwapError::Io(_))),
            "DB failure during update_desktop_profile_flag must surface as Err, got {:?}",
            result
        );
    }

    #[tokio::test]
    async fn test_desktop_switch_db_active_pointer_failure_propagates() {
        // Drop ONLY the state table: snapshot writes files (OK), accounts
        // UPDATE for update_desktop_profile_flag works (if used), but
        // set_active_desktop INSERTs into state → fails → switch() returns Err.
        let _lock = crate::testing::lock_data_dir();
        let _env_dir = setup_test_data_dir();
        let data_dir = _env_dir.path().join("Claude");
        fs::create_dir_all(&data_dir).unwrap();

        let (store, _db) = test_store();
        let tgt = make_account("tgt@example.com");
        store.insert(&tgt).unwrap();

        let tgt_profile = crate::paths::desktop_profile_dir(tgt.uuid);
        fs::create_dir_all(&tgt_profile).unwrap();
        fs::write(tgt_profile.join("config.json"), "tgt-cfg").unwrap();

        store.corrupt_state_table_for_test();

        let platform = MockDesktopPlatform::new(&data_dir, &["config.json"]);
        // outgoing_id=None so we skip update_desktop_profile_flag — the only
        // DB write is set_active_desktop, which needs the state table.
        let result = switch(&platform, &store, None, tgt.uuid, true).await;
        assert!(
            matches!(result, Err(DesktopSwapError::Io(_))),
            "DB failure during set_active_desktop must surface as Err, got {:?}",
            result
        );
    }

    #[tokio::test]
    async fn test_desktop_switch_with_windows_items() {
        let _lock = crate::testing::lock_data_dir();
        let _env_dir = setup_test_data_dir();
        let data_dir = _env_dir.path().join("Claude");
        fs::create_dir_all(&data_dir).unwrap();
        populate_windows_data_dir(&data_dir);

        let (store, _db_dir) = test_store();
        let out = make_account("out@example.com");
        let tgt = make_account("tgt@example.com");
        store.insert(&out).unwrap();
        store.insert(&tgt).unwrap();

        // Pre-populate target profile with distinct Network/Cookies content.
        let tgt_profile = crate::paths::desktop_profile_dir(tgt.uuid);
        fs::create_dir_all(tgt_profile.join("Network")).unwrap();
        fs::write(tgt_profile.join("Network/Cookies"), "tgt-cookies").unwrap();
        fs::write(tgt_profile.join("config.json"), "tgt-config").unwrap();

        let mut platform = MockDesktopPlatform::new(&data_dir, WINDOWS_ITEMS);
        platform.running.store(true, Ordering::SeqCst);

        switch(&platform, &store, Some(out.uuid), tgt.uuid, true)
            .await
            .unwrap();

        // Outgoing snapshot captured nested items.
        let out_profile = crate::paths::desktop_profile_dir(out.uuid);
        assert_eq!(
            fs::read_to_string(out_profile.join("Network/Cookies")).unwrap(),
            "file-Network/Cookies"
        );
        // Data dir now has target's content — nested path correctly restored.
        assert_eq!(
            fs::read_to_string(data_dir.join("Network/Cookies")).unwrap(),
            "tgt-cookies"
        );
        assert_eq!(
            fs::read_to_string(data_dir.join("config.json")).unwrap(),
            "tgt-config"
        );
        assert_eq!(
            store.active_desktop_uuid().unwrap(),
            Some(tgt.uuid.to_string())
        );
    }
}
