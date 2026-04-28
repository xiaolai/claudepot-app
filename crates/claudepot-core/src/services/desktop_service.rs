//! Claude Desktop service — reconcile, adopt, clear, sync.
//!
//! Phase 1 shipped reconcile-only. Phase 3 adds the three mutators
//! that change disk and DB state. Every mutator here takes a
//! [`crate::desktop_identity::VerifiedIdentity`] so the type system
//! enforces "no mutation on candidate identity" (Codex D5-1 mitigation).

use crate::account::AccountStore;
use crate::desktop_backend::swap;
use crate::desktop_backend::DesktopPlatform;
use crate::desktop_identity::VerifiedIdentity;
use crate::desktop_lock;
use crate::paths;
use chrono::Utc;
use uuid::Uuid;

/// Result of a `reconcile_flags` pass.
#[derive(Debug, Default, Clone)]
pub struct ReconcileOutcome {
    /// One entry per account whose `has_desktop_profile` flag flipped.
    pub flag_flips: Vec<FlagFlip>,
    /// True when `state.active_desktop` pointed at a UUID that does
    /// not (or no longer) correspond to a registered account, and we
    /// cleared it.
    pub orphan_pointer_cleared: bool,
}

#[derive(Debug, Clone)]
pub struct FlagFlip {
    pub email: String,
    pub uuid: Uuid,
    pub new_value: bool,
}

/// Bring `accounts.has_desktop_profile` into alignment with the
/// filesystem. Every mismatch flips the DB flag to match on-disk
/// truth (the flag is a cached view of the snapshot dir's
/// existence). Also clears `state.active_desktop` when it points at
/// a UUID that no longer has a registered account.
///
/// Idempotent — a clean state returns an empty [`ReconcileOutcome`].
/// Safe to run in the hot path of `account_list` (all writes are
/// O(1) per changed row).
pub fn reconcile_flags(store: &AccountStore) -> Result<ReconcileOutcome, rusqlite::Error> {
    let accounts = store.list()?;
    let mut flips = Vec::new();

    for a in &accounts {
        let on_disk = paths::desktop_profile_dir(a.uuid).exists();
        if a.has_desktop_profile != on_disk {
            store.update_desktop_profile_flag(a.uuid, on_disk)?;
            flips.push(FlagFlip {
                email: a.email.clone(),
                uuid: a.uuid,
                new_value: on_disk,
            });
        }
    }

    // Orphan pointer: state.active_desktop holds a UUID but no
    // registered account matches. Possible after an out-of-band DB
    // edit, a test harness glitch, or a race where an account was
    // removed while the pointer update failed.
    let orphan_cleared = match store.active_desktop_uuid()? {
        Some(uuid_str) => {
            let is_orphan = match Uuid::parse_str(&uuid_str) {
                // Un-parseable UUIDs are definitionally orphaned.
                Err(_) => true,
                Ok(u) => !accounts.iter().any(|a| a.uuid == u),
            };
            if is_orphan {
                store.clear_active_desktop()?;
                true
            } else {
                false
            }
        }
        None => false,
    };

    Ok(ReconcileOutcome {
        flag_flips: flips,
        orphan_pointer_cleared: orphan_cleared,
    })
}

// ---------------------------------------------------------------------------
// Phase 3 — adopt / clear / sync (require VerifiedIdentity)
// ---------------------------------------------------------------------------

/// Outcome of a successful [`switch`].
#[derive(Debug, Clone)]
pub struct SwitchOutcome {
    /// Email of the target account that Desktop is now bound to.
    pub email: String,
    /// Email of the previously-active account, if any. `None` when
    /// no `active_desktop` pointer was set going in.
    pub outgoing_email: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum SwitchError {
    #[error("{email} has no Desktop profile yet \u{2014} sign in via the Desktop app first")]
    NoSnapshot { email: String },
    #[error("target account {0} is not registered")]
    NotFound(Uuid),
    #[error("Desktop is not supported on this platform")]
    Unsupported,
    #[error(transparent)]
    Swap(#[from] crate::error::DesktopSwapError),
    #[error("store: {0}")]
    Store(String),
}

/// Switch the active Desktop profile to `target_uuid`.
///
/// Wraps [`crate::desktop_backend::swap::switch`] with the snapshot
/// preflight + outgoing-id lookup that callers (Tauri command, CLI)
/// previously inlined. Snapshot existence is checked BEFORE quitting
/// Desktop — a missing snapshot must not leave the user staring at a
/// quit Desktop with no recovery path.
///
/// Errors:
///   - [`SwitchError::NotFound`] — `target_uuid` is not in the store.
///   - [`SwitchError::NoSnapshot`] — target exists but has never been
///     adopted; user must sign into Desktop and adopt first.
///   - [`SwitchError::Swap`] — anything from the underlying swap
///     pipeline (DPAPI mismatch, quit timeout, file copy I/O, etc.).
///   - [`SwitchError::Store`] — DB read failed during target lookup.
pub async fn switch(
    platform: &dyn DesktopPlatform,
    store: &AccountStore,
    target_uuid: Uuid,
    no_launch: bool,
) -> Result<SwitchOutcome, SwitchError> {
    // Target lookup.
    let target = store
        .find_by_uuid(target_uuid)
        .map_err(|e| SwitchError::Store(e.to_string()))?
        .ok_or(SwitchError::NotFound(target_uuid))?;

    // Snapshot preflight — runs BEFORE swap::switch so a missing
    // snapshot can't quit Desktop. Error message must match the
    // user-visible string the Tauri command used to print verbatim.
    if !paths::desktop_profile_dir(target_uuid).exists() {
        return Err(SwitchError::NoSnapshot {
            email: target.email.clone(),
        });
    }

    // Outgoing pointer — best-effort. A malformed value behaves like
    // no pointer (the swap layer treats `None` as "first switch").
    let outgoing_id = store
        .active_desktop_uuid()
        .map_err(|e| SwitchError::Store(e.to_string()))?
        .and_then(|s| Uuid::parse_str(&s).ok());

    let outgoing_email = match outgoing_id {
        Some(u) => store
            .find_by_uuid(u)
            .map_err(|e| SwitchError::Store(e.to_string()))?
            .map(|a| a.email),
        None => None,
    };

    swap::switch(platform, store, outgoing_id, target_uuid, no_launch).await?;

    Ok(SwitchOutcome {
        email: target.email,
        outgoing_email,
    })
}

/// Outcome of a successful [`adopt_current`].
#[derive(Debug, Clone)]
pub struct AdoptOutcome {
    pub account_email: String,
    pub captured_items: usize,
    pub size_bytes: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum AdoptError {
    #[error("live Desktop identity is {actual}, not {expected}")]
    IdentityMismatch { expected: String, actual: String },
    #[error("target account {0} is not registered")]
    NotFound(Uuid),
    #[error("target already has a profile; pass overwrite=true to replace")]
    ProfileExists,
    #[error("Desktop is not supported on this platform")]
    Unsupported,
    #[error("data_dir missing or unreadable")]
    DataDirUnreadable,
    #[error("swap error: {0}")]
    Swap(#[from] crate::error::DesktopSwapError),
    #[error("store: {0}")]
    Store(String),
    #[error("lock: {0}")]
    Lock(#[from] desktop_lock::DesktopLockError),
    #[error("sidecar write failed: {0}")]
    Sidecar(String),
}

/// Adopt the live Desktop session into `target_uuid`'s snapshot
/// directory. Gated on a [`VerifiedIdentity`] whose email matches the
/// target account's stored email.
///
/// Flow:
/// 1. Acquire the Desktop operation lock.
/// 2. Load the target account; verify emails match.
/// 3. Enforce overwrite policy on the profile dir.
/// 4. Quit Desktop (if running).
/// 5. Snapshot live data_dir → profile dir.
/// 6. Write profile.toml sidecar (D17).
/// 7. Update has_desktop_profile + active_desktop.
/// 8. Launch Desktop.
pub async fn adopt_current(
    platform: &dyn DesktopPlatform,
    store: &AccountStore,
    target_uuid: Uuid,
    verified: &VerifiedIdentity,
    overwrite: bool,
) -> Result<AdoptOutcome, AdoptError> {
    // Target + identity checks run BEFORE prelude — they're cheap,
    // don't need the lock, and failing fast keeps us from quitting
    // Desktop for an op that can't succeed.
    let target = store
        .find_by_uuid(target_uuid)
        .map_err(|e| AdoptError::Store(e.to_string()))?
        .ok_or(AdoptError::NotFound(target_uuid))?;

    if !verified.email().eq_ignore_ascii_case(&target.email) {
        return Err(AdoptError::IdentityMismatch {
            expected: target.email,
            actual: verified.email().to_string(),
        });
    }

    let profile_dir = paths::desktop_profile_dir(target_uuid);
    if profile_dir.exists() && !overwrite {
        return Err(AdoptError::ProfileExists);
    }

    // Shared prelude: acquires lock, resolves data_dir, quits Desktop.
    let prelude = desktop_prelude(platform).await.map_err(|e| match e {
        crate::error::DesktopSwapError::Lock(lock_err) => AdoptError::Lock(lock_err),
        crate::error::DesktopSwapError::NotInstalled => AdoptError::DataDirUnreadable,
        other => AdoptError::Swap(other),
    })?;
    let data_dir = &prelude.data_dir;
    let items = prelude.items;

    // Overwrite-safe: stage the old profile into a temp dir first so
    // we can roll back if any subsequent step fails. Kept alive for
    // the whole commit sequence (snapshot + sidecar + DB flags) so
    // no intermediate failure can leave the user with a partial
    // profile and no recovery artifact.
    let stash = if profile_dir.exists() {
        let staging = tempfile::Builder::new()
            .prefix("claudepot-adopt-prev-")
            .tempdir()
            .map_err(|e| AdoptError::Sidecar(format!("staging dir: {e}")))?;
        let dst = staging.path().join("profile");
        crate::fs_utils::copy_dir_recursive(&profile_dir, &dst)
            .map_err(|e| AdoptError::Sidecar(format!("stashing old profile: {e}")))?;
        std::fs::remove_dir_all(&profile_dir)
            .map_err(|e| AdoptError::Sidecar(format!("purging old profile: {e}")))?;
        Some((staging, dst))
    } else {
        None
    };

    // Restore helper — every failure path between here and the final
    // store write funnels through this so a partial profile_dir never
    // coexists with the stash contents. Always clears profile_dir
    // first so copying back starts from a clean state.
    let restore_stash = |stash: &Option<(tempfile::TempDir, std::path::PathBuf)>| {
        let _ = std::fs::remove_dir_all(&profile_dir);
        if let Some((_, staged)) = stash.as_ref() {
            let _ = crate::fs_utils::copy_dir_recursive(staged, &profile_dir);
        }
    };

    if let Err(e) = swap::snapshot(data_dir, target_uuid, items) {
        restore_stash(&stash);
        return Err(e.into());
    }

    // Count + size for the outcome DTO. Cheap because we just wrote
    // the files — everything is already in the filesystem cache.
    let (captured_items, size_bytes) = measure_profile(&profile_dir);

    // Sidecar (D17) — captured metadata that survives dir mtime churn.
    if let Err(e) = write_sidecar(
        &profile_dir,
        SidecarMeta {
            captured_at: Utc::now(),
            captured_from_email: verified.email().to_string(),
            captured_verified: true,
            claudepot_version: env!("CARGO_PKG_VERSION").to_string(),
            platform: std::env::consts::OS.to_string(),
            session_items: items.iter().map(|s| s.to_string()).collect(),
        },
    ) {
        restore_stash(&stash);
        return Err(AdoptError::Sidecar(e.to_string()));
    }

    if let Err(e) = store.update_desktop_profile_flag(target_uuid, true) {
        restore_stash(&stash);
        return Err(AdoptError::Store(e.to_string()));
    }
    if let Err(e) = store.set_active_desktop(target_uuid) {
        // Disk revert matters for DB consistency:
        //   - No stash (first-time adopt): we just flipped the flag
        //     from false→true for this uuid; `profile_dir` will be
        //     empty after `restore_stash`, so the flag must go back
        //     to false to match disk.
        //   - Had stash (overwrite adopt): the flag was already true
        //     *for a valid old profile* before this call; after
        //     restore_stash the old profile is back on disk, so the
        //     true flag is still correct. Leaving it alone avoids
        //     creating DB-vs-disk drift in the opposite direction.
        if stash.is_none() {
            let _ = store.update_desktop_profile_flag(target_uuid, false);
        }
        restore_stash(&stash);
        return Err(AdoptError::Store(e.to_string()));
    }

    // Commit successful — drop the stash (TempDir auto-cleans).
    drop(stash);

    // Relaunch Desktop so the user's workflow is uninterrupted. Best-
    // effort — a launch failure doesn't invalidate the snapshot,
    // which is the durable artifact.
    let _ = platform.launch().await;

    Ok(AdoptOutcome {
        account_email: target.email,
        captured_items,
        size_bytes,
    })
}

/// Outcome of a successful [`clear_session`].
#[derive(Debug, Clone)]
pub struct ClearOutcome {
    pub email: Option<String>,
    pub snapshot_kept: bool,
    pub items_deleted: usize,
}

#[derive(Debug, thiserror::Error)]
pub enum ClearError {
    #[error("Desktop is not supported on this platform")]
    Unsupported,
    #[error("data_dir missing — Desktop is already signed out")]
    DataDirMissing,
    #[error("swap error: {0}")]
    Swap(#[from] crate::error::DesktopSwapError),
    #[error("filesystem: {0}")]
    Fs(String),
    #[error("store: {0}")]
    Store(String),
    #[error("lock: {0}")]
    Lock(#[from] desktop_lock::DesktopLockError),
}

/// Sign Desktop out — by default stashes the current session into
/// the active account's snapshot dir first. Does NOT relaunch
/// Desktop (the intent is "leave me signed out").
///
/// Windows postcondition: deletes every [`DesktopPlatform::session_items`]
/// entry. On Windows, nested items under `Network/` are removed; the
/// parent `Network/` directory is removed iff it ends up empty. All
/// non-session files (caches, extensions) are retained.
pub async fn clear_session(
    platform: &dyn DesktopPlatform,
    store: &AccountStore,
    keep_snapshot: bool,
) -> Result<ClearOutcome, ClearError> {
    // Shared prelude: acquires lock, resolves data_dir, quits Desktop.
    let prelude = desktop_prelude(platform).await.map_err(|e| match e {
        crate::error::DesktopSwapError::Lock(lock_err) => ClearError::Lock(lock_err),
        crate::error::DesktopSwapError::NotInstalled => ClearError::DataDirMissing,
        other => ClearError::Swap(other),
    })?;
    let data_dir = &prelude.data_dir;
    let items = prelude.items;

    // Look up the active account so we know whose snapshot (if any)
    // to stash. The pointer being None is non-fatal — the user may
    // have signed in outside of Claudepot. But a store *error* (DB
    // unreadable, lock poisoned, schema mismatch) must propagate:
    // silently treating it as "no active account" would skip the
    // snapshot AND still proceed with the destructive delete.
    let active = match store
        .active_desktop_uuid()
        .map_err(|e| ClearError::Store(e.to_string()))?
        .and_then(|s| Uuid::parse_str(&s).ok())
    {
        Some(u) => store
            .find_by_uuid(u)
            .map_err(|e| ClearError::Store(e.to_string()))?,
        None => None,
    };

    // Snapshot-before-delete when requested + feasible.
    let snapshot_kept = if keep_snapshot {
        if let Some(acct) = &active {
            swap::snapshot(data_dir, acct.uuid, items)?;
            store
                .update_desktop_profile_flag(acct.uuid, true)
                .map_err(|e| ClearError::Store(e.to_string()))?;
            // Sidecar uses the account's stored email, not a verified
            // live identity — clear_session doesn't require
            // VerifiedIdentity (the intent is to sign out, identity
            // is secondary).
            let profile_dir = paths::desktop_profile_dir(acct.uuid);
            let _ = write_sidecar(
                &profile_dir,
                SidecarMeta {
                    captured_at: Utc::now(),
                    captured_from_email: acct.email.clone(),
                    captured_verified: false, // not /profile-confirmed
                    claudepot_version: env!("CARGO_PKG_VERSION").to_string(),
                    platform: std::env::consts::OS.to_string(),
                    session_items: items.iter().map(|s| s.to_string()).collect(),
                },
            );
            true
        } else {
            false
        }
    } else {
        false
    };

    // Delete every session item from data_dir.
    let items_deleted = delete_session_items(data_dir, items)?;

    // Clean up the Network/ parent on Windows if empty.
    prune_empty_parents(data_dir, items);

    // Clear the active pointer regardless of snapshot outcome —
    // Desktop is no longer signed in.
    store
        .clear_active_desktop()
        .map_err(|e| ClearError::Store(e.to_string()))?;

    Ok(ClearOutcome {
        email: active.map(|a| a.email),
        snapshot_kept,
        items_deleted,
    })
}

/// Startup / window-focus sync. Never mutates disk. Returns a
/// [`SyncOutcome`] describing what Claudepot should do next (UI
/// layer surfaces adoption banners, refreshes the pointer cache,
/// etc.).
#[derive(Debug, Clone)]
pub enum SyncOutcome {
    /// No Desktop session or the platform is unsupported.
    NoLive,
    /// Live identity matches a registered account AND that account
    /// has a snapshot on disk. Nothing to do — pointer cache is
    /// already correct.
    Verified { email: String },
    /// Live identity matches a registered account but no snapshot
    /// exists yet. UI surfaces a "Bind current Desktop session to
    /// <email>" banner.
    AdoptionAvailable { email: String },
    /// Live identity does not match any registered account. UI
    /// offers "Register <email>" (Add + Adopt flow).
    Stranger { email: String },
    /// The only signal we got was a fast-path candidate. UI treats
    /// as "possible match — verify" (no mutation on this tier).
    CandidateOnly { email: String },
}

pub async fn sync_from_current(
    platform: &dyn DesktopPlatform,
    store: &AccountStore,
) -> Result<SyncOutcome, crate::desktop_identity::DesktopIdentityError> {
    use crate::desktop_identity::{
        probe_live_identity_async, DefaultProfileFetcher, ProbeMethod, ProbeOptions,
    };
    let fetcher = DefaultProfileFetcher;
    match probe_live_identity_async(platform, store, ProbeOptions { strict: true }, &fetcher).await
    {
        Ok(None) => Ok(SyncOutcome::NoLive),
        Ok(Some(id)) => {
            // strict=true guarantees Decrypted tier.
            debug_assert!(id.probe_method == ProbeMethod::Decrypted);
            let matched = store.find_by_email(&id.email).ok().flatten();
            match matched {
                None => Ok(SyncOutcome::Stranger { email: id.email }),
                Some(acct) => {
                    let on_disk = paths::desktop_profile_dir(acct.uuid).exists();
                    if on_disk {
                        // Cache the pointer — sync is supposed to
                        // keep active_desktop in step with reality.
                        let _ = store.set_active_desktop(acct.uuid);
                        Ok(SyncOutcome::Verified { email: acct.email })
                    } else {
                        Ok(SyncOutcome::AdoptionAvailable { email: acct.email })
                    }
                }
            }
        }
        // Slow path failed — fall back to surfacing the fast-path
        // candidate as "possible match." UI must treat as unverified.
        Err(crate::desktop_identity::DesktopIdentityError::Key(_))
        | Err(crate::desktop_identity::DesktopIdentityError::Decrypt(_))
        | Err(crate::desktop_identity::DesktopIdentityError::TokenParse(_))
        | Err(crate::desktop_identity::DesktopIdentityError::ProfileFetch(_)) => {
            let candidate = crate::desktop_identity::probe_live_identity(
                platform,
                store,
                ProbeOptions::default(),
            );
            match candidate {
                Ok(Some(c)) => Ok(SyncOutcome::CandidateOnly { email: c.email }),
                _ => Ok(SyncOutcome::NoLive),
            }
        }
        Err(crate::desktop_identity::DesktopIdentityError::NotSignedIn) => Ok(SyncOutcome::NoLive),
        Err(e) => Err(e),
    }
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn measure_profile(profile_dir: &std::path::Path) -> (usize, u64) {
    let mut count = 0usize;
    let mut size = 0u64;
    if let Ok(entries) = std::fs::read_dir(profile_dir) {
        for entry in entries.flatten() {
            count += 1;
            size = size.saturating_add(dir_or_file_size(&entry.path()));
        }
    }
    (count, size)
}

fn dir_or_file_size(p: &std::path::Path) -> u64 {
    match std::fs::metadata(p) {
        Err(_) => 0,
        Ok(md) if md.is_file() => md.len(),
        Ok(_) => std::fs::read_dir(p)
            .map(|it| {
                it.flatten()
                    .map(|e| dir_or_file_size(&e.path()))
                    .sum::<u64>()
            })
            .unwrap_or(0),
    }
}

fn delete_session_items(data_dir: &std::path::Path, items: &[&str]) -> Result<usize, ClearError> {
    let mut deleted = 0;
    for item in items {
        let p = data_dir.join(item);
        if !p.exists() {
            continue;
        }
        let result = if p.is_dir() {
            std::fs::remove_dir_all(&p)
        } else {
            std::fs::remove_file(&p)
        };
        match result {
            Ok(()) => deleted += 1,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(ClearError::Fs(format!("{}: {e}", p.display()))),
        }
    }
    Ok(deleted)
}

fn prune_empty_parents(data_dir: &std::path::Path, items: &[&str]) {
    use std::collections::BTreeSet;
    let mut candidates = BTreeSet::new();
    for item in items {
        if let Some(parent) = std::path::Path::new(item).parent() {
            if parent.as_os_str().is_empty() {
                continue;
            }
            candidates.insert(parent.to_path_buf());
        }
    }
    for parent in candidates {
        let full = data_dir.join(&parent);
        if std::fs::read_dir(&full)
            .map(|mut it| it.next().is_none())
            .unwrap_or(false)
        {
            let _ = std::fs::remove_dir(&full);
        }
    }
}

/// Shared preamble for Desktop mutators — acquires the operation
/// lock, resolves `data_dir`, and quits Desktop if running. Each
/// mutator previously inlined this shape; centralising it makes
/// the precondition contract explicit and avoids drift.
///
/// Consciously NOT the full `execute_plan` collapse plan-v2 §Phase 7
/// proposed: Codex D5-4 flagged that as HIGH blast-radius because
/// `switch` / `adopt_current` / `clear_session` have distinct
/// rollback semantics. Each mutator keeps its own body + rollback —
/// only the ~15 lines of shared setup live here.
#[doc(hidden)]
pub(crate) struct DesktopPrelude<'a> {
    pub data_dir: std::path::PathBuf,
    pub items: &'a [&'a str],
    // Holds the flock for the full lifetime of the mutator. Dropped
    // when the mutator returns, releasing the lock.
    _lock: crate::desktop_lock::DesktopLockGuard,
}

/// Acquire the Desktop operation lock, resolve data_dir, quit
/// Desktop if running. Returns the prelude on success.
///
/// Error discrimination:
///   - [`crate::desktop_lock::DesktopLockError::Held`] → another
///     Claudepot op is already in progress.
///   - [`crate::error::DesktopSwapError::NotInstalled`] → platform
///     has no data_dir OR data_dir doesn't exist on disk.
pub(crate) async fn desktop_prelude<'a>(
    platform: &'a dyn DesktopPlatform,
) -> Result<DesktopPrelude<'a>, crate::error::DesktopSwapError> {
    use crate::error::DesktopSwapError;

    let _lock = crate::desktop_lock::try_acquire()?;

    let data_dir = platform.data_dir().ok_or(DesktopSwapError::NotInstalled)?;
    if !data_dir.exists() {
        return Err(DesktopSwapError::NotInstalled);
    }
    let items = platform.session_items();

    if platform.is_running().await {
        tracing::info!("Desktop op prelude — quitting Claude Desktop");
        platform.quit().await?;
    }

    Ok(DesktopPrelude {
        data_dir,
        items,
        _lock,
    })
}

/// Windows DPAPI invalidation pre-check.
///
/// Tier 2-B (2026-04-23) — upgraded from the Phase 6 keyring-only
/// probe to a ciphertext-level probe. Codex follow-up review flagged
/// the keyring-only version: it missed the subtler case where the
/// CURRENT `Local State` is freshly re-encrypted (machine migration,
/// Windows password reset that regenerated the DPAPI master key)
/// but the STORED SNAPSHOT's ciphertext is still bound to the OLD
/// key. The keyring unwraps fine — it's the stored blobs that are
/// dead.
///
/// Algorithm (Windows only):
///
/// 1. If no `profile_dir/config.json` exists → Ok(true). Nothing to
///    validate; the snapshot is empty or signed-out.
/// 2. Read `oauth:tokenCache` from that file. Missing → Ok(true)
///    (the snapshot was captured from a signed-out Desktop).
/// 3. Fetch the live `safe_storage_secret` (current DPAPI master key).
/// 4. Attempt AES-GCM decrypt of the stored ciphertext with the
///    current key. Success → Ok(true). AES failure → Ok(false): the
///    ciphertext is dead, profile is invalidated.
/// 5. If `safe_storage_secret` itself errors with a DPAPI-shaped
///    failure → Ok(false) (any stored ciphertext is un-decryptable
///    by definition — the previous keyring-level probe).
///
/// macOS: no-op — safeStorage keys are stable across reboots and
/// sessions, so the invalidation mode doesn't exist.
pub async fn check_profile_dpapi_valid(
    platform: &dyn DesktopPlatform,
    account_uuid: Uuid,
) -> Result<bool, crate::desktop_backend::DesktopKeyError> {
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (platform, account_uuid);
        Ok(true)
    }
    #[cfg(target_os = "windows")]
    {
        let profile_dir = paths::desktop_profile_dir(account_uuid);
        let cfg_path = profile_dir.join("config.json");
        if !cfg_path.exists() {
            return Ok(true);
        }
        let raw = match std::fs::read_to_string(&cfg_path) {
            Ok(s) => s,
            Err(_) => return Ok(true), // unreadable — don't block
        };
        let Ok(cfg) = serde_json::from_str::<serde_json::Value>(&raw) else {
            return Ok(true); // malformed stored snapshot — not a DPAPI issue
        };
        let Some(token_b64) = cfg.get("oauth:tokenCache").and_then(|v| v.as_str()) else {
            return Ok(true); // snapshot of a signed-out Desktop
        };

        // Live key — any DPAPI-shape error here means the snapshot
        // can't possibly be decrypted under this Windows session.
        let secret = match platform.safe_storage_secret().await {
            Ok(k) => k,
            Err(crate::desktop_backend::DesktopKeyError::DpapiFailed(_))
            | Err(crate::desktop_backend::DesktopKeyError::LocalState(_)) => {
                return Ok(false);
            }
            Err(e) => return Err(e),
        };

        // Ciphertext-level check: try the real decrypt path. This
        // catches the subtle "new keyring but old snapshot" case
        // that the keyring-only probe missed.
        //
        // Discriminate by error kind — only AES failure implies the
        // live DPAPI key can't decrypt the stored ciphertext (the
        // actual invalidation signal). Base64 / version / format
        // errors mean the stored snapshot itself is corrupt; that's
        // a different failure mode. Surface it as an error to the
        // caller so the UI can report "snapshot corrupt — re-bind"
        // instead of silently returning true and letting a bad
        // snapshot proceed.
        use crate::desktop_backend::crypto::DecryptError;
        use crate::desktop_backend::DesktopKeyError;
        match crate::desktop_backend::crypto::windows::decrypt(token_b64, &secret) {
            Ok(_) => Ok(true),
            Err(DecryptError::Aes) => Ok(false),
            Err(DecryptError::Base64(msg)) => Err(DesktopKeyError::LocalState(format!(
                "snapshot token base64 malformed: {msg}"
            ))),
            Err(DecryptError::BadFormat(msg)) => Err(DesktopKeyError::LocalState(format!(
                "snapshot token format invalid: {msg}"
            ))),
            Err(DecryptError::UnknownVersion(tag)) => Err(DesktopKeyError::LocalState(format!(
                "snapshot token uses unsupported envelope tag: {tag:?}"
            ))),
        }
    }
}

#[derive(Debug, serde::Serialize)]
struct SidecarMeta {
    captured_at: chrono::DateTime<chrono::Utc>,
    captured_from_email: String,
    captured_verified: bool,
    claudepot_version: String,
    platform: String,
    session_items: Vec<String>,
}

fn write_sidecar(profile_dir: &std::path::Path, meta: SidecarMeta) -> std::io::Result<()> {
    // JSON, not TOML, so we don't pull in another dep. The plan uses
    // the name `profile.toml` for familiarity but the actual encoding
    // is JSON — both are human-readable and the parse side is
    // serde-backed either way.
    std::fs::create_dir_all(profile_dir)?;
    let path = profile_dir.join("claudepot.profile.json");
    let body = serde_json::to_vec_pretty(&meta).map_err(std::io::Error::other)?;
    std::fs::write(path, body)
}

#[cfg(test)]
#[path = "desktop_service_tests.rs"]
mod tests;
