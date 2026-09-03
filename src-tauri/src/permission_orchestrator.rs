//! Permission-grant orchestrator — bridges
//! `claudepot_core::permission` to the Tauri runtime.
//!
//! Unlike `rotation_orchestrator`, this holds **no managed state**:
//! grants live entirely on disk (`~/.claudepot/permission-grants.json`)
//! and are cheap to reload each tick. Three jobs, every
//! `usage_snapshot::run_tick`:
//!
//! - drop grants whose deadline has passed and say so
//!   (`permission-reverted`, outcome `expired`);
//! - finish migrating schema-1 grants — put the `bypassPermissions`
//!   they wrote into `.claude/settings.local.json` back, then carry
//!   their deadline over as a hook grant — and tell the user once;
//! - keep Claude Code's `PreToolUse` hook entry in step with the file:
//!   present and pointing at this binary while a grant is live, gone
//!   otherwise. Doing this every tick is also what repairs the entry
//!   after the binary moves.
//!
//! Zero overhead when no grants exist — `tick` returns after one cheap
//! file read and an uninstall that finds nothing to remove.

use chrono::{DateTime, Utc};
use claudepot_core::notification_log::{NotificationKind, NotificationSource};
use claudepot_core::permission::grants::{GrantsFile, LegacySettingsGrant};
use claudepot_core::permission::settings::{
    clear_default_mode, read_default_mode, write_default_mode, PermissionSettingsError,
};
use claudepot_core::permission::{eval, hook, store as permission_store, PermissionMode};
use serde::Serialize;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};
use tauri::{AppHandle, Emitter, Manager};

/// Serializes every load → mutate → save sequence on
/// `permission-grants.json` within this process, **and the hook
/// reconcile that follows it**. The writers are [`tick`] (every 5 min
/// via `usage_snapshot::run_tick`) and the `permission_grant` /
/// `permission_revert` / `permission_extend` commands; without a shared
/// lock an interleaved tick could save its older snapshot over a
/// just-upserted grant, or read "no grants" and take the hook entry out
/// from under a grant a command had just written. `atomic_write` only
/// prevents torn files, not lost updates.
///
/// Read-only loads (`permission_list` / `permission_get`) don't take
/// the lock: the atomic file replace means they see either the old or
/// the new snapshot, and a stale read has no persistence to lose.
///
/// The CLI never writes this file — the hook verb reads it and nothing
/// else — so an intra-process mutex is the whole fix.
static GRANTS_FILE_LOCK: Mutex<()> = Mutex::new(());

/// Acquire the grants-file lock, recovering from poison (a panic in
/// one writer must not disable grant persistence for the app's
/// lifetime). Hold the returned guard across the entire load → mutate
/// → save → reconcile sequence, and never across an `.await`.
pub fn grants_file_guard() -> MutexGuard<'static, ()> {
    claudepot_core::sync::recover_lock(&GRANTS_FILE_LOCK, "permission grants file")
}

/// Make Claude Code's `PreToolUse` entry agree with `file` at `now`,
/// pointing at this binary. Returns whether it is now installed.
///
/// `current_exe` and not a looked-up path: this binary is the one
/// carrying the verb, so the hook cannot point at a Claudepot that is
/// not this one. Callers hold [`grants_file_guard`].
pub fn reconcile_hook(file: &GrantsFile, now: DateTime<Utc>) -> Result<bool, String> {
    let binary = std::env::current_exe().map_err(|e| format!("cannot locate this binary: {e}"))?;
    hook::reconcile(file, now, &binary).map_err(|e| e.to_string())
}

/// What happened when a legacy grant's settings write was undone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevertOutcome {
    /// The layer still held the granted mode; it was restored to
    /// `previous_mode` (or the key was cleared).
    Reverted,
    /// The layer no longer holds the granted mode — the user changed
    /// the setting themselves after the grant. We don't clobber their
    /// change; the record is just dropped.
    SkippedUserChanged,
}

/// Undo one schema-1 grant's settings write, with a safety check: only
/// restore `previous_mode` if the settings layer *still* holds exactly
/// `granted_mode`. If the user has since changed the setting by hand,
/// leave their value alone and report
/// [`RevertOutcome::SkippedUserChanged`].
pub fn revert_legacy(
    grant: &LegacySettingsGrant,
) -> Result<RevertOutcome, PermissionSettingsError> {
    let root = Path::new(&grant.project_path);
    let current = read_default_mode(&grant.layer.settings_file(root))?;
    if user_changed_layer(current.as_ref(), &grant.granted_mode) {
        return Ok(RevertOutcome::SkippedUserChanged);
    }
    match &grant.previous_mode {
        Some(prev) => write_default_mode(grant.layer, root, prev)?,
        None => clear_default_mode(grant.layer, root)?,
    }
    Ok(RevertOutcome::Reverted)
}

/// Pure skip-user-changed comparison: the layer no longer holds
/// exactly the granted mode (changed value, or key removed) means the
/// user took over and the revert must not clobber their choice.
fn user_changed_layer(current: Option<&PermissionMode>, granted: &PermissionMode) -> bool {
    current != Some(granted)
}

/// One migration pass over `file.legacy`, with the revert injected so
/// tests run it against a fixture instead of real settings files.
///
/// A record whose revert succeeds (or was skipped because the user
/// already changed the key) is promoted to a hook grant carrying its
/// deadline. A record whose revert fails is kept for the next tick, up
/// to [`LegacySettingsGrant::MAX_REVERT_ATTEMPTS`]; the attempt that
/// crosses that line drops it and reports it, so a malformed settings
/// file costs three warnings and one notice rather than one warning
/// every five minutes for the app's lifetime. The key CC ignores
/// anyway, so what is left behind is litter, not an elevation.
fn migrate_legacy<F, E>(file: &mut GrantsFile, now: DateTime<Utc>, revert: F) -> Migration
where
    F: Fn(&LegacySettingsGrant) -> Result<RevertOutcome, E>,
    E: std::fmt::Display,
{
    let mut out = Migration::default();
    for legacy in file.legacy.clone() {
        match revert(&legacy) {
            Ok(_) => {
                let promoted = file.promote_legacy(&legacy.project_path, now);
                out.migrated.push((legacy.project_path, promoted.is_some()));
            }
            Err(e) => {
                let attempts = legacy.revert_attempts + 1;
                tracing::warn!(
                    project = %legacy.project_path,
                    attempt = attempts,
                    error = %e,
                    "permission_orchestrator: legacy grant revert failed"
                );
                if attempts >= LegacySettingsGrant::MAX_REVERT_ATTEMPTS {
                    file.legacy
                        .retain(|l| l.project_path != legacy.project_path);
                    out.given_up.push(legacy.project_path);
                } else if let Some(live) = file
                    .legacy
                    .iter_mut()
                    .find(|l| l.project_path == legacy.project_path)
                {
                    live.revert_attempts = attempts;
                    out.retrying = true;
                }
            }
        }
    }
    out
}

/// What one migration pass did. Every field non-empty means the file
/// changed and there is something to tell the user.
#[derive(Debug, Default, PartialEq, Eq)]
struct Migration {
    /// `(project, still_granted)` — the settings key is back to what
    /// it was; `true` when the deadline had not passed and a hook grant
    /// now carries it.
    migrated: Vec<(String, bool)>,
    /// Reverts that failed for the last time; the settings file needs a
    /// person.
    given_up: Vec<String>,
    /// A revert failed but will be retried next tick.
    retrying: bool,
}

impl Migration {
    fn changed_file(&self) -> bool {
        !self.migrated.is_empty() || !self.given_up.is_empty() || self.retrying
    }
    fn worth_a_notice(&self) -> bool {
        !self.migrated.is_empty() || !self.given_up.is_empty()
    }
}

/// Drive one cycle. Called from `usage_snapshot::run_tick` after the
/// snapshot is written. A real I/O failure on load skips the tick
/// (rather than treating it as "no grants" and taking the hook entry
/// out from under a live grant).
pub async fn tick(app: &AppHandle) {
    // Exclude the grant commands for the whole read-modify-write
    // cycle (this function never awaits, so holding a sync guard is
    // safe and keeps the future `Send`).
    let _guard = grants_file_guard();
    let loaded = match permission_store::load_outcome() {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!(error = %e, "permission_orchestrator: grants load failed; skipping tick");
            return;
        }
    };
    let mut file = loaded.value;

    // Corruption is loud for this store: the file just got recovered
    // to empty, which withdrew every grant and dropped any legacy
    // revert obligation. Surface it BEFORE the empty early return
    // below, which a recovered file always hits. `corrupt_grant_copies`
    // also catches a recovery that happened in an earlier process;
    // that cross-restart scan runs once per process.
    let recovered_now = loaded.recovery.is_some();
    let first_scan = !CORRUPTION_SCAN_DONE.swap(true, Ordering::Relaxed);
    if recovered_now || first_scan {
        maybe_notify_grants_corruption(app, recovered_now);
    }

    let now = Utc::now();
    let mut changed = false;

    let migration = migrate_legacy(&mut file, now, revert_legacy);
    changed |= migration.changed_file();
    if migration.worth_a_notice() {
        notify_migration(app, &migration);
    }

    for grant in eval::expired_grants(&file, now)
        .into_iter()
        .cloned()
        .collect::<Vec<_>>()
    {
        file.remove(&grant.project_path);
        changed = true;
        emit_reverted(app, &grant.project_path);
    }

    if changed {
        if let Err(e) = permission_store::save(&file) {
            tracing::warn!(error = %e, "permission_orchestrator: grants save failed");
        }
    }

    // Last, and unconditionally: the entry follows the file. A grant
    // that outlived an app upgrade gets its `command` re-pointed here.
    if let Err(e) = reconcile_hook(&file, now) {
        tracing::warn!(error = %e, "permission_orchestrator: hook reconcile failed");
    }
}

/// One cross-restart corruption scan per process — set on the first
/// tick. A fresh in-tick recovery (`recovered_now`) bypasses this
/// gate, so a corruption event mid-run is still surfaced.
static CORRUPTION_SCAN_DONE: AtomicBool = AtomicBool::new(false);

/// Surface a grants-file corruption recovery to the user through the
/// existing notification mechanism — an OS banner plus a bell-log
/// entry. The store is loud on corruption (see `permission::store`):
/// the recovered-to-empty file silently withdrew every grant, and a
/// user who granted eight hours of unattended work should hear that
/// the prompts are back rather than discover it.
fn maybe_notify_grants_corruption(app: &AppHandle, recovered_now: bool) {
    let prior_copies = !permission_store::corrupt_grant_copies().is_empty();
    let Some((title, body)) = corruption_notice(recovered_now, prior_copies) else {
        return;
    };
    notify(app, NotificationKind::Error, title, body);
}

/// Pure decision: should a corruption recovery be surfaced?
///
/// A recovery observed by *this* load is always surfaced. Stale
/// forensic copies from an earlier process are not: the grants they
/// held are gone either way, the hook fails closed without them, and
/// re-nagging on every launch about a file the user may have already
/// looked at would be noise.
fn corruption_notice(recovered_now: bool, _prior_corrupt_copies: bool) -> Option<(String, String)> {
    use crate::i18n::tr;
    if !recovered_now {
        return None;
    }
    Some((tr("permission.corruptTitle"), tr("permission.corruptBody")))
}

/// Tell the user, once per migration pass, what happened to the grants
/// a schema-1 Claudepot wrote into settings files. Both halves in one
/// entry: which projects had their key put back (and which of those
/// are still granted, now through the hook), and which settings files
/// could not be repaired.
fn notify_migration(app: &AppHandle, m: &Migration) {
    let (title, body) = migration_notice(m);
    notify(app, NotificationKind::Notice, title, body);
}

/// Pure: the notice text for a migration pass.
fn migration_notice(m: &Migration) -> (String, String) {
    use crate::i18n::{tr, tr1};
    let title = tr("permission.migratedTitle");
    let mut parts = Vec::new();
    let regranted: Vec<&str> = m
        .migrated
        .iter()
        .filter(|(_, live)| *live)
        .map(|(p, _)| p.as_str())
        .collect();
    let lapsed: Vec<&str> = m
        .migrated
        .iter()
        .filter(|(_, live)| !*live)
        .map(|(p, _)| p.as_str())
        .collect();
    if !regranted.is_empty() {
        parts.push(tr1(
            "permission.migratedRegranted",
            "list",
            &regranted.join(", "),
        ));
    }
    if !lapsed.is_empty() {
        parts.push(tr1("permission.migratedLapsed", "list", &lapsed.join(", ")));
    }
    if !m.given_up.is_empty() {
        parts.push(tr1(
            "permission.migratedGivenUp",
            "list",
            &m.given_up.join(", "),
        ));
    }
    (title, parts.join(" "))
}

fn notify(app: &AppHandle, kind: NotificationKind, title: String, body: String) {
    {
        use tauri_plugin_notification::NotificationExt;
        if let Err(e) = app
            .notification()
            .builder()
            .title(&title)
            .body(&body)
            .show()
        {
            tracing::warn!(error = %e, "permission_orchestrator: OS notification failed");
        }
    }
    if let Some(log) = app.try_state::<crate::commands::notification::NotificationLogState>() {
        if let Err(e) = log.log.append(
            NotificationSource::Os,
            kind,
            title,
            body,
            serde_json::Value::Null,
        ) {
            tracing::warn!(error = %e, "permission_orchestrator: log append failed");
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PermissionRevertedPayload {
    project_path: String,
    /// Always `"expired"` today: a hook grant reaches its deadline and
    /// is dropped. Kept as a string so a second outcome can be added
    /// without changing the event's shape.
    outcome: String,
}

fn emit_reverted(app: &AppHandle, project_path: &str) {
    let payload = PermissionRevertedPayload {
        project_path: project_path.to_string(),
        outcome: "expired".into(),
    };
    if let Err(e) = app.emit(crate::events::PERMISSION_REVERTED, payload) {
        tracing::warn!(error = %e, "permission_orchestrator: emit reverted failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use claudepot_core::permission::grants::Grant;
    use claudepot_core::settings_writer::SettingsLayer;

    /// One test covers both lock properties (a single test because
    /// the static is shared — a separate poison test could race a
    /// separate exclusivity test under the parallel runner):
    ///
    /// 1. the guard is exclusive — the lost-update race between the
    ///    orchestrator tick and the grant commands is what it exists
    ///    to close;
    /// 2. a panic while holding it must not disable grant
    ///    persistence for the app lifetime — `grants_file_guard`
    ///    recovers from poison.
    #[test]
    fn test_grants_file_guard_exclusive_and_poison_recoverable() {
        {
            let _g = grants_file_guard();
            assert!(
                matches!(
                    GRANTS_FILE_LOCK.try_lock(),
                    Err(std::sync::TryLockError::WouldBlock)
                ),
                "second acquire must block while the guard is held"
            );
        }

        // Poison the static via a panicking holder thread.
        let join = std::thread::spawn(|| {
            let _g = GRANTS_FILE_LOCK.lock().unwrap();
            panic!("intentional poison");
        });
        let _ = join.join();
        assert!(
            GRANTS_FILE_LOCK.is_poisoned(),
            "setup: lock must be poisoned"
        );

        // The guard helper must still hand out the lock.
        let _g = grants_file_guard();
    }

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, 1, 12, 0, 0).unwrap()
    }

    fn legacy(
        path: &str,
        previous: Option<PermissionMode>,
        expires: Option<DateTime<Utc>>,
    ) -> LegacySettingsGrant {
        LegacySettingsGrant {
            project_path: path.to_string(),
            layer: SettingsLayer::LocalProject,
            granted_mode: PermissionMode::BypassPermissions,
            previous_mode: previous,
            granted_at: Utc.with_ymd_and_hms(2026, 6, 1, 10, 0, 0).unwrap(),
            expires_at: expires,
            revert_attempts: 0,
        }
    }

    fn with_legacy(legacy: Vec<LegacySettingsGrant>) -> GrantsFile {
        GrantsFile {
            legacy,
            ..GrantsFile::default()
        }
    }

    // ── revert_legacy — the three branches, on real settings files ─

    #[test]
    fn test_revert_legacy_restores_previous_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write_default_mode(
            SettingsLayer::LocalProject,
            root,
            &PermissionMode::BypassPermissions,
        )
        .unwrap();
        let g = legacy(root.to_str().unwrap(), Some(PermissionMode::Default), None);
        assert_eq!(revert_legacy(&g).unwrap(), RevertOutcome::Reverted);
        let after = read_default_mode(&SettingsLayer::LocalProject.settings_file(root)).unwrap();
        assert_eq!(after, Some(PermissionMode::Default));
    }

    #[test]
    fn test_revert_legacy_clears_key_when_no_previous_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write_default_mode(
            SettingsLayer::LocalProject,
            root,
            &PermissionMode::BypassPermissions,
        )
        .unwrap();
        let g = legacy(root.to_str().unwrap(), None, None);
        assert_eq!(revert_legacy(&g).unwrap(), RevertOutcome::Reverted);
        let after = read_default_mode(&SettingsLayer::LocalProject.settings_file(root)).unwrap();
        assert_eq!(after, None, "key must be cleared, not set to a mode");
    }

    #[test]
    fn test_revert_legacy_skips_when_user_changed_layer() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        // User hand-set the layer to plain `default` after the grant.
        write_default_mode(SettingsLayer::LocalProject, root, &PermissionMode::Default).unwrap();
        let g = legacy(root.to_str().unwrap(), Some(PermissionMode::Plan), None);
        assert_eq!(
            revert_legacy(&g).unwrap(),
            RevertOutcome::SkippedUserChanged
        );
        // Their value must be left alone — not clobbered with `Plan`.
        let after = read_default_mode(&SettingsLayer::LocalProject.settings_file(root)).unwrap();
        assert_eq!(after, Some(PermissionMode::Default));
    }

    #[test]
    fn test_revert_legacy_errors_on_a_malformed_settings_file() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let path = SettingsLayer::LocalProject.settings_file(root);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"{ not json").unwrap();
        let g = legacy(root.to_str().unwrap(), None, None);
        assert!(revert_legacy(&g).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"{ not json");
    }

    // ── user_changed_layer — the pure comparison ───────────────────

    #[test]
    fn test_user_changed_layer() {
        assert!(!user_changed_layer(
            Some(&PermissionMode::BypassPermissions),
            &PermissionMode::BypassPermissions
        ));
        assert!(user_changed_layer(
            Some(&PermissionMode::Default),
            &PermissionMode::BypassPermissions
        ));
        assert!(user_changed_layer(None, &PermissionMode::BypassPermissions));
    }

    // ── migrate_legacy — promotion, bounded retries ────────────────

    #[test]
    fn a_reverted_legacy_grant_becomes_a_hook_grant_with_its_deadline() {
        let later = now() + chrono::Duration::hours(2);
        let mut f = with_legacy(vec![legacy("/p/a", None, Some(later))]);
        let m = migrate_legacy(&mut f, now(), |_| Ok::<_, String>(RevertOutcome::Reverted));
        assert_eq!(m.migrated, vec![("/p/a".to_string(), true)]);
        assert!(m.given_up.is_empty());
        assert!(f.legacy.is_empty());
        assert_eq!(
            f.grants,
            vec![Grant {
                project_path: "/p/a".into(),
                granted_at: Utc.with_ymd_and_hms(2026, 6, 1, 10, 0, 0).unwrap(),
                expires_at: Some(later),
            }]
        );
        assert!(m.changed_file() && m.worth_a_notice());
    }

    #[test]
    fn a_user_changed_key_still_promotes_the_grant() {
        // The user took the key out by hand; the grant they asked for
        // is still what they asked for, now honoured by the hook.
        let mut f = with_legacy(vec![legacy("/p/a", None, None)]);
        let m = migrate_legacy(&mut f, now(), |_| {
            Ok::<_, String>(RevertOutcome::SkippedUserChanged)
        });
        assert_eq!(m.migrated, vec![("/p/a".to_string(), true)]);
        assert_eq!(f.grants.len(), 1);
        assert!(f.grants[0].expires_at.is_none(), "sticky stays sticky");
    }

    #[test]
    fn an_already_expired_legacy_grant_is_reverted_and_reported_as_lapsed() {
        let earlier = now() - chrono::Duration::hours(1);
        let mut f = with_legacy(vec![legacy("/p/a", None, Some(earlier))]);
        let m = migrate_legacy(&mut f, now(), |_| Ok::<_, String>(RevertOutcome::Reverted));
        assert_eq!(m.migrated, vec![("/p/a".to_string(), false)]);
        assert!(f.grants.is_empty());
        assert!(f.legacy.is_empty());
    }

    #[test]
    fn a_failed_revert_is_kept_and_counted_until_the_third_failure() {
        let mut f = with_legacy(vec![legacy("/p/broken", None, None)]);
        for attempt in 1..LegacySettingsGrant::MAX_REVERT_ATTEMPTS {
            let m = migrate_legacy(&mut f, now(), |_| Err::<RevertOutcome, _>("malformed"));
            assert!(m.migrated.is_empty() && m.given_up.is_empty());
            assert!(m.retrying && m.changed_file() && !m.worth_a_notice());
            assert_eq!(f.legacy.len(), 1, "kept for retry");
            assert_eq!(f.legacy[0].revert_attempts, attempt);
            assert!(
                f.grants.is_empty(),
                "never promoted while the key is still there"
            );
        }
        let m = migrate_legacy(&mut f, now(), |_| Err::<RevertOutcome, _>("malformed"));
        assert_eq!(m.given_up, vec!["/p/broken".to_string()]);
        assert!(
            f.legacy.is_empty(),
            "given up: dropped, not retried forever"
        );
        assert!(
            f.grants.is_empty(),
            "and NOT promoted — the key was never put back"
        );
        assert!(m.worth_a_notice());
    }

    #[test]
    fn one_broken_settings_file_does_not_block_the_others() {
        let mut f = with_legacy(vec![
            legacy("/p/broken", None, None),
            legacy("/p/fine", None, None),
        ]);
        let m = migrate_legacy(&mut f, now(), |l| {
            if l.project_path == "/p/broken" {
                Err("malformed".to_string())
            } else {
                Ok(RevertOutcome::Reverted)
            }
        });
        assert_eq!(m.migrated, vec![("/p/fine".to_string(), true)]);
        assert_eq!(f.legacy.len(), 1);
        assert_eq!(f.legacy[0].project_path, "/p/broken");
        assert_eq!(f.grants.len(), 1);
        assert_eq!(f.grants[0].project_path, "/p/fine");
    }

    #[test]
    fn an_empty_legacy_list_is_a_no_op() {
        let mut f = GrantsFile::default();
        let m = migrate_legacy(&mut f, now(), |_| Ok::<_, String>(RevertOutcome::Reverted));
        assert_eq!(m, Migration::default());
        assert!(!m.changed_file());
    }

    // ── notices — pure text ────────────────────────────────────────

    #[test]
    fn test_corruption_notice_fires_only_for_a_fresh_recovery() {
        assert!(corruption_notice(false, false).is_none());
        assert!(
            corruption_notice(false, true).is_none(),
            "stale forensic copies alone are not worth a banner"
        );
        let (title, body) = corruption_notice(true, false).unwrap();
        assert!(title.contains("unreadable"), "title={title}");
        assert!(body.contains("withdrawn"), "body={body}");
    }

    #[test]
    fn test_migration_notice_names_each_group_once() {
        let m = Migration {
            migrated: vec![("/p/live".into(), true), ("/p/lapsed".into(), false)],
            given_up: vec!["/p/broken".into()],
            retrying: false,
        };
        let (title, body) = migration_notice(&m);
        assert!(title.contains("2.1.257"), "title={title}");
        assert!(body.contains("/p/live"), "body={body}");
        assert!(body.contains("/p/lapsed"), "body={body}");
        assert!(body.contains("/p/broken"), "body={body}");
        // The three groups are told apart by their sentence, not by
        // the reader guessing.
        let live_at = body.find("/p/live").unwrap();
        let lapsed_at = body.find("/p/lapsed").unwrap();
        let broken_at = body.find("/p/broken").unwrap();
        assert!(live_at < lapsed_at && lapsed_at < broken_at, "body={body}");
    }
}
