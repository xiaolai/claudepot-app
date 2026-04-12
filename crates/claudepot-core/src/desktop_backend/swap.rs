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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::DesktopSwapError;
    use crate::testing::{DATA_DIR_LOCK, setup_test_data_dir, test_store, make_account};
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
        assert_eq!(fs::read_to_string(profile.join("config.json")).unwrap(), "v2");
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

        switch(&platform, &store, Some(out_acct.uuid), tgt_acct.uuid, false).await.unwrap();

        // Verify quit was called
        assert!(platform.quit_called.load(Ordering::SeqCst));
        // Verify launch was called
        assert!(platform.launch_called.load(Ordering::SeqCst));
        // Verify data_dir now has target's config
        assert_eq!(fs::read_to_string(data_dir.join("config.json")).unwrap(), "target-config");
        // Verify outgoing was snapshotted
        let out_profile = crate::paths::desktop_profile_dir(out_acct.uuid);
        assert_eq!(fs::read_to_string(out_profile.join("config.json")).unwrap(), "outgoing-config");
        // Verify active desktop pointer
        assert_eq!(store.active_desktop_uuid().unwrap(), Some(tgt_acct.uuid.to_string()));
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
        switch(&platform, &store, None, tgt.uuid, true).await.unwrap();

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
        switch(&platform, &store, None, tgt.uuid, false).await.unwrap();

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
        switch(&platform, &store, None, tgt.uuid, false).await.unwrap();

        // Data dir should have target's config
        assert_eq!(fs::read_to_string(data_dir.join("config.json")).unwrap(), "cfg");
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
}
