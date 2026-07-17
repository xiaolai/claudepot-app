//! Permission-grant orchestrator — bridges
//! `claudepot_core::permission` to the Tauri runtime.
//!
//! Unlike `rotation_orchestrator`, this holds **no managed state**:
//! grants live entirely on disk (`~/.claudepot/permission-grants.json`)
//! and are cheap to reload each tick. The orchestrator is two free
//! functions:
//!
//! - [`tick`] — called from `usage_snapshot::run_tick`; reverts every
//!   grant whose deadline has passed and drops it from the file.
//! - [`revert_grant`] — the safety-checked revert, also used directly
//!   by the `permission_revert` command.
//!
//! Zero overhead when no grants exist — `tick` returns after one
//! cheap file read.

use chrono::{DateTime, Utc};
use claudepot_core::breaker;
use claudepot_core::notification_log::{NotificationKind, NotificationSource};
use claudepot_core::permission::grants::{Grant, GrantsFile};
use claudepot_core::permission::settings::{
    clear_default_mode, read_default_mode, resolve_default_mode, write_default_mode,
    PermissionSettingsError,
};
use claudepot_core::permission::{eval, store as permission_store, PermissionMode};
use serde::Serialize;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};
use tauri::{AppHandle, Emitter, Manager};

/// Serializes every load → mutate → save sequence on
/// `permission-grants.json` within this process. The writers are
/// [`tick`] (every 5 min via `usage_snapshot::run_tick`) and the
/// `permission_grant` / `permission_revert` / `permission_extend`
/// commands; without a shared lock an interleaved tick could save
/// its older snapshot over a just-upserted grant, leaving the
/// project's settings elevated with no managing grant record — the
/// same fail-open as the corruption case, never auto-reverted.
/// `atomic_write` only prevents torn files, not lost updates.
///
/// Read-only loads (`permission_list` / `permission_get` /
/// `current_dto`) don't take the lock: the atomic file replace means
/// they see either the old or the new snapshot, and a stale read
/// has no persistence to lose.
///
/// The CLI never touches this file, so an intra-process mutex is
/// the whole fix — no file lock needed.
static GRANTS_FILE_LOCK: Mutex<()> = Mutex::new(());

/// Acquire the grants-file lock, recovering from poison (a panic in
/// one writer must not disable grant persistence — and auto-revert —
/// for the app's lifetime). Hold the returned guard across the
/// entire load → mutate → save sequence, and never across an
/// `.await`.
pub fn grants_file_guard() -> MutexGuard<'static, ()> {
    claudepot_core::sync::recover_lock(&GRANTS_FILE_LOCK, "permission grants file")
}

/// What happened when a grant's deadline was reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RevertOutcome {
    /// The layer still held the granted mode; it was restored to
    /// `previous_mode` (or the key was cleared).
    Reverted,
    /// The layer no longer holds the granted mode — the user changed
    /// the setting themselves after the grant. We don't clobber their
    /// change; the grant is just dropped.
    SkippedUserChanged,
}

/// Revert one grant, with a safety check: only restore `previous_mode`
/// if the settings layer *still* holds exactly `granted_mode`. If the
/// user has since changed the setting by hand, leave their value
/// alone and report [`RevertOutcome::SkippedUserChanged`].
pub fn revert_grant(grant: &Grant) -> Result<RevertOutcome, PermissionSettingsError> {
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
/// exactly the granted mode (changed value, or key removed) means
/// the user took over and the revert must not clobber their choice.
fn user_changed_layer(current: Option<&PermissionMode>, granted: &PermissionMode) -> bool {
    current != Some(granted)
}

/// Pure stale-sweep decision: grants whose project's LocalProject
/// layer no longer holds `granted_mode` (per the injected
/// `local_value` resolver) — the user hand-edited settings since the
/// grant was created, so the on-disk record is no longer managing
/// anything. [`tick`] injects `resolve_default_mode`; tests inject a
/// closure over a fixture map.
///
/// A resolver error (unreadable settings for that project) skips
/// that ONE grant — never marked stale, never aborting the sweep.
/// Anything stronger would fail-open: one malformed settings file
/// would block the expired-grant revert loop for every other
/// project. The unreadable project's own revert attempt fails
/// downstream and advances its circuit breaker.
fn stale_grant_paths<F, E>(grants: &[Grant], local_value: F) -> Vec<String>
where
    F: Fn(&str) -> Result<Option<PermissionMode>, E>,
{
    grants
        .iter()
        .filter(|g| match local_value(&g.project_path) {
            Ok(current) => current.as_ref() != Some(&g.granted_mode),
            Err(_) => false,
        })
        .map(|g| g.project_path.clone())
        .collect()
}

/// Pure breaker advance for a failed revert: returns the advanced
/// ledger plus whether this failure is the one that newly crosses
/// the threshold. `trips_on_next_failure` is checked *before* the
/// record so the trip event fires exactly once — the same idiom the
/// rotation orchestrator uses.
fn advance_breaker_on_failure(
    prev: &breaker::FailureLedger,
    now: DateTime<Utc>,
) -> (breaker::FailureLedger, bool) {
    let newly_tripped = breaker::trips_on_next_failure(prev);
    (breaker::record_failure(prev, now), newly_tripped)
}

/// Drive one expiration cycle. Called from `usage_snapshot::run_tick`
/// after the snapshot is written. Loads grants, reverts the expired
/// ones, and saves the trimmed file. A real I/O failure on load skips
/// the tick (rather than treating it as "no grants" and never
/// reverting); a per-grant revert failure advances that grant's
/// consecutive-failure circuit breaker and leaves it in the file to
/// retry next tick.
///
/// A grant whose breaker is *tripped* — repeated revert failures —
/// is skipped entirely: not reverted, not retried, just left in the
/// file flagged. The breaker's cooldown lets one probe retry through
/// later. The breaker arithmetic is pure `claudepot_core::breaker`;
/// this function only wires it.
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

    // Corruption is fail-open for this store: the on-disk record is
    // the ONLY thing obliging us to revert a `bypassPermissions`
    // elevation, and it just got recovered to empty. Surface it —
    // BEFORE the grants-empty early return below, which a recovered
    // file always hits. `corrupt_grant_copies` also catches a
    // recovery that happened in an earlier process (a command-path
    // load, a previous app run); that cross-restart scan runs once
    // per process.
    let recovered_now = loaded.recovery.is_some();
    let first_scan = !CORRUPTION_SCAN_DONE.swap(true, Ordering::Relaxed);
    if recovered_now || first_scan {
        maybe_notify_grants_corruption(app, &file, recovered_now);
    }

    if file.grants.is_empty() {
        return;
    }

    let now = Utc::now();

    // Sweep stale records first: any grant whose `granted_mode` no
    // longer matches the project's LocalProject layer value means the
    // user hand-edited settings since the grant was created. Time-
    // boxed grants self-heal via `revert_grant`'s `skipped_user_changed`
    // path on the expired-revert loop below, but sticky grants would
    // otherwise linger on disk indefinitely. Drop them here so disk
    // state matches what `current_dto::filter_stale` already hides
    // from the UI.
    let stale_paths = stale_grant_paths(&file.grants, |path| {
        resolve_default_mode(std::path::Path::new(path))
            .map(|state| state.local_project_value)
            .inspect_err(|e| {
                tracing::warn!(
                    project_path = %path,
                    error = %e,
                    "permission_orchestrator: settings unreadable; skipping stale check for this grant"
                );
            })
    });
    let mut changed = false;
    for path in &stale_paths {
        if file.remove(path).is_some() {
            changed = true;
        }
    }

    let expired_paths: Vec<String> = eval::expired_grants(&file, now)
        .into_iter()
        .map(|g| g.project_path.clone())
        .collect();
    if expired_paths.is_empty() {
        if changed {
            if let Err(e) = permission_store::save(&file) {
                tracing::warn!(error = %e, "permission_orchestrator: stale sweep save failed");
            }
        }
        return;
    }

    for path in &expired_paths {
        // Re-fetch the grant from `file` each iteration so the
        // breaker mutations below land on the live struct.
        let grant = match file.find(path) {
            Some(g) => g.clone(),
            None => continue,
        };

        // Circuit-breaker gate: a grant that has failed to revert
        // THRESHOLD times in a row is quarantined — skip it until
        // the cooldown lets a probe through. Without this, a
        // permanently un-revertable grant (settings file deleted,
        // permission lost) retries every 5 minutes forever.
        if breaker::is_tripped(&grant.breaker_ledger(), now) {
            continue;
        }

        match revert_grant(&grant) {
            Ok(outcome) => {
                // Success removes the grant — its breaker state goes
                // with it. (Probe retries that succeed land here.)
                file.remove(path);
                changed = true;
                emit_reverted(app, &grant, outcome);
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    project = %path,
                    "permission_orchestrator: revert failed; advancing circuit breaker"
                );
                // Advance the breaker on the live grant and persist.
                // `advance_breaker_on_failure` checks the trip
                // transition *before* the record so the trip event
                // fires exactly once, on the failure that crosses
                // the threshold.
                if let Some(live) = file.find_mut(path) {
                    let (ledger, newly_tripped) =
                        advance_breaker_on_failure(&live.breaker_ledger(), now);
                    live.set_breaker_ledger(ledger);
                    changed = true;
                    if newly_tripped {
                        emit_breaker_tripped(app, &grant, ledger.consecutive);
                    }
                }
            }
        }
    }

    if changed {
        if let Err(e) = permission_store::save(&file) {
            tracing::warn!(error = %e, "permission_orchestrator: grants save failed");
        }
    }
}

/// One cross-restart corruption scan per process — set on the first
/// tick. A fresh in-tick recovery (`recovered_now`) bypasses this
/// gate, so a corruption event mid-run is still surfaced.
static CORRUPTION_SCAN_DONE: AtomicBool = AtomicBool::new(false);

/// Surface a grants-file corruption recovery to the user. The grants
/// store is fail-open on corruption (see `permission::store` module
/// docs): the recovered-to-empty file silently ends every revert
/// obligation, so the user must hear about it through the existing
/// notification mechanism — an OS banner plus a bell-log entry
/// listing the projects whose `settings.local.json` still holds
/// `bypassPermissions` with no managing grant.
fn maybe_notify_grants_corruption(app: &AppHandle, file: &GrantsFile, recovered_now: bool) {
    let prior_copies = !permission_store::corrupt_grant_copies().is_empty();
    if !recovered_now && !prior_copies {
        return;
    }
    let elevated = elevated_unmanaged_projects(file);
    let Some((title, body)) = corruption_notice(recovered_now, prior_copies, &elevated) else {
        return;
    };

    // OS banner — this is a "your safety net is gone" condition;
    // worth OS-level prominence like the P0 categories get.
    {
        use tauri_plugin_notification::NotificationExt;
        if let Err(e) = app
            .notification()
            .builder()
            .title(&title)
            .body(&body)
            .show()
        {
            tracing::warn!(error = %e, "permission_orchestrator: corruption OS notification failed");
        }
    }

    // Bell log so the event survives past the banner.
    if let Some(log) = app.try_state::<crate::commands::notification::NotificationLogState>() {
        if let Err(e) = log.log.append(
            NotificationSource::Os,
            NotificationKind::Error,
            title,
            body,
            serde_json::Value::Null,
        ) {
            tracing::warn!(error = %e, "permission_orchestrator: corruption log append failed");
        }
    }
}

/// Pure decision: should a corruption recovery be surfaced, and with
/// what message?
///
/// - A recovery observed by *this* load is always surfaced — the
///   user's revert obligations were just dropped, even if no project
///   is currently elevated.
/// - Stale forensic copies from an *earlier* process are only worth
///   a notification while some project still holds an unmanaged
///   `bypassPermissions`; once the user has re-granted or reverted
///   everything by hand, re-nagging on every launch would be noise.
fn corruption_notice(
    recovered_now: bool,
    prior_corrupt_copies: bool,
    elevated_bypass_projects: &[String],
) -> Option<(String, String)> {
    let stale_copies_worth_notice = prior_corrupt_copies && !elevated_bypass_projects.is_empty();
    if !recovered_now && !stale_copies_worth_notice {
        return None;
    }
    let title = "Permission grants file was unreadable".to_string();
    let body = if elevated_bypass_projects.is_empty() {
        "The permission-grants file was corrupt and has been moved aside. \
         No project currently holds bypassPermissions."
            .to_string()
    } else {
        let shown: Vec<&str> = elevated_bypass_projects
            .iter()
            .map(|s| s.as_str())
            .take(3)
            .collect();
        let more = elevated_bypass_projects.len() - shown.len();
        let list = if more > 0 {
            format!("{} and {more} more", shown.join(", "))
        } else {
            shown.join(", ")
        };
        format!(
            "The permission-grants file was corrupt and has been moved aside. \
             These projects hold bypassPermissions and will NOT auto-revert: \
             {list}. Re-grant or revert them by hand."
        )
    };
    Some((title, body))
}

/// Projects whose LocalProject layer holds `bypassPermissions` but
/// that have no grant in `file` — after a corruption recovery these
/// are the elevations nobody is managing. Scans the CC project list;
/// only invoked on the (rare) corruption path.
fn elevated_unmanaged_projects(file: &GrantsFile) -> Vec<String> {
    let cfg = claudepot_core::paths::claude_config_dir();
    let projects = match claudepot_core::project::list_projects(&cfg) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, "permission_orchestrator: project scan for corruption notice failed");
            return Vec::new();
        }
    };
    let mut modes = Vec::with_capacity(projects.len());
    for p in projects {
        match resolve_default_mode(Path::new(&p.original_path)) {
            Ok(state) => modes.push((p.original_path, state.local_project_value)),
            Err(e) => tracing::warn!(
                project_path = %p.original_path,
                error = %e,
                "permission_orchestrator: skipping unreadable project settings"
            ),
        }
    }
    filter_elevated_unmanaged(modes, file)
}

/// Pure filter behind [`elevated_unmanaged_projects`]: keep paths
/// whose layer value is `bypassPermissions` AND that have no grant
/// record in `file`.
fn filter_elevated_unmanaged(
    modes: Vec<(String, Option<PermissionMode>)>,
    file: &GrantsFile,
) -> Vec<String> {
    modes
        .into_iter()
        .filter(|(path, mode)| {
            *mode == Some(PermissionMode::BypassPermissions) && file.find(path).is_none()
        })
        .map(|(path, _)| path)
        .collect()
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PermissionRevertedPayload {
    project_path: String,
    /// The mode the project is back to (`previous_mode`, or the CC
    /// default wire string when the grant cleared the key).
    reverted_to: String,
    /// `"reverted"` or `"skipped_user_changed"`.
    outcome: String,
}

fn emit_reverted(app: &AppHandle, grant: &Grant, outcome: RevertOutcome) {
    let reverted_to = grant
        .previous_mode
        .as_ref()
        .map(|m| m.as_wire_str().to_string())
        .unwrap_or_else(|| "default".to_string());
    let payload = PermissionRevertedPayload {
        project_path: grant.project_path.clone(),
        reverted_to,
        outcome: match outcome {
            RevertOutcome::Reverted => "reverted".into(),
            RevertOutcome::SkippedUserChanged => "skipped_user_changed".into(),
        },
    };
    if let Err(e) = app.emit(crate::events::PERMISSION_REVERTED, payload) {
        tracing::warn!(error = %e, "permission_orchestrator: emit reverted failed");
    }
}

/// `permission-breaker-tripped` payload — a grant's auto-revert kept
/// failing, so its circuit breaker quarantined it. Emitted once, on
/// the failure that crosses the threshold.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct PermissionBreakerTrippedPayload {
    project_path: String,
    /// Number of consecutive revert failures that tripped the breaker.
    consecutive_failures: u32,
}

fn emit_breaker_tripped(app: &AppHandle, grant: &Grant, consecutive_failures: u32) {
    let payload = PermissionBreakerTrippedPayload {
        project_path: grant.project_path.clone(),
        consecutive_failures,
    };
    if let Err(e) = app.emit(crate::events::PERMISSION_BREAKER_TRIPPED, payload) {
        tracing::warn!(error = %e, "permission_orchestrator: emit breaker-tripped failed");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One test covers both lock properties (a single test because
    /// the static is shared — a separate poison test could race a
    /// separate exclusivity test under the parallel runner):
    ///
    /// 1. the guard is exclusive — the lost-update race between the
    ///    orchestrator tick and the grant commands is what it exists
    ///    to close;
    /// 2. a panic while holding it must not disable grant
    ///    persistence (and auto-revert) for the app lifetime —
    ///    `grants_file_guard` recovers from poison.
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

    use chrono::TimeZone;
    use claudepot_core::settings_writer::SettingsLayer;

    fn grant(path: &str, granted: PermissionMode, previous: Option<PermissionMode>) -> Grant {
        Grant {
            project_path: path.to_string(),
            layer: SettingsLayer::LocalProject,
            granted_mode: granted,
            previous_mode: previous,
            granted_at: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
            expires_at: Some(Utc.timestamp_opt(1_700_007_200, 0).unwrap()),
            consecutive_failures: 0,
            last_failure_at: None,
        }
    }

    fn grants_file(grants: Vec<Grant>) -> GrantsFile {
        GrantsFile {
            grants,
            ..GrantsFile::default()
        }
    }

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 6, 1, 12, 0, 0).unwrap()
    }

    // ── revert_grant — the three branches, on real settings files ──

    #[test]
    fn test_revert_grant_restores_previous_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write_default_mode(
            SettingsLayer::LocalProject,
            root,
            &PermissionMode::BypassPermissions,
        )
        .unwrap();
        let g = grant(
            root.to_str().unwrap(),
            PermissionMode::BypassPermissions,
            Some(PermissionMode::Default),
        );
        assert_eq!(revert_grant(&g).unwrap(), RevertOutcome::Reverted);
        let after = read_default_mode(&SettingsLayer::LocalProject.settings_file(root)).unwrap();
        assert_eq!(after, Some(PermissionMode::Default));
    }

    #[test]
    fn test_revert_grant_clears_key_when_no_previous_mode() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write_default_mode(
            SettingsLayer::LocalProject,
            root,
            &PermissionMode::BypassPermissions,
        )
        .unwrap();
        let g = grant(
            root.to_str().unwrap(),
            PermissionMode::BypassPermissions,
            None,
        );
        assert_eq!(revert_grant(&g).unwrap(), RevertOutcome::Reverted);
        let after = read_default_mode(&SettingsLayer::LocalProject.settings_file(root)).unwrap();
        assert_eq!(after, None, "key must be cleared, not set to a mode");
    }

    #[test]
    fn test_revert_grant_skips_when_user_changed_layer() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        // User hand-set the layer to plain `default` after the grant.
        write_default_mode(SettingsLayer::LocalProject, root, &PermissionMode::Default).unwrap();
        let g = grant(
            root.to_str().unwrap(),
            PermissionMode::BypassPermissions,
            Some(PermissionMode::Plan),
        );
        assert_eq!(revert_grant(&g).unwrap(), RevertOutcome::SkippedUserChanged);
        // Their value must be left alone — not clobbered with `Plan`.
        let after = read_default_mode(&SettingsLayer::LocalProject.settings_file(root)).unwrap();
        assert_eq!(after, Some(PermissionMode::Default));
    }

    // ── user_changed_layer — the pure comparison ───────────────────

    #[test]
    fn test_user_changed_layer_matching_mode_is_not_changed() {
        assert!(!user_changed_layer(
            Some(&PermissionMode::BypassPermissions),
            &PermissionMode::BypassPermissions
        ));
    }

    #[test]
    fn test_user_changed_layer_different_mode_is_changed() {
        assert!(user_changed_layer(
            Some(&PermissionMode::Default),
            &PermissionMode::BypassPermissions
        ));
    }

    #[test]
    fn test_user_changed_layer_cleared_key_is_changed() {
        assert!(user_changed_layer(None, &PermissionMode::BypassPermissions));
    }

    // ── stale_grant_paths — sweep decision with injected resolver ──

    #[test]
    fn test_stale_grant_paths_keeps_matching_grant() {
        let grants = vec![grant("/p/a", PermissionMode::BypassPermissions, None)];
        let stale = stale_grant_paths(&grants, |_| {
            Ok::<_, String>(Some(PermissionMode::BypassPermissions))
        });
        assert!(stale.is_empty());
    }

    #[test]
    fn test_stale_grant_paths_flags_hand_edited_value() {
        let grants = vec![
            grant("/p/a", PermissionMode::BypassPermissions, None),
            grant("/p/b", PermissionMode::BypassPermissions, None),
        ];
        // /p/a was hand-reverted to default; /p/b still matches.
        let stale = stale_grant_paths(&grants, |path| {
            Ok::<_, String>(if path == "/p/a" {
                Some(PermissionMode::Default)
            } else {
                Some(PermissionMode::BypassPermissions)
            })
        });
        assert_eq!(stale, vec!["/p/a".to_string()]);
    }

    #[test]
    fn test_stale_grant_paths_flags_cleared_key() {
        let grants = vec![grant("/p/a", PermissionMode::BypassPermissions, None)];
        let stale = stale_grant_paths(&grants, |_| Ok::<_, String>(None));
        assert_eq!(stale, vec!["/p/a".to_string()]);
    }

    #[test]
    fn unreadable_settings_skips_that_grant_not_the_sweep() {
        // One project's unreadable settings must neither mark that
        // grant stale nor abort evaluation of the others — the
        // whole-tick abort was the fail-open that let a single
        // malformed settings file block every project's auto-revert.
        let grants = vec![
            grant("/p/broken", PermissionMode::BypassPermissions, None),
            grant("/p/edited", PermissionMode::BypassPermissions, None),
        ];
        let stale = stale_grant_paths(&grants, |path| {
            if path == "/p/broken" {
                Err("settings unreadable".to_string())
            } else {
                Ok(Some(PermissionMode::Default))
            }
        });
        assert_eq!(stale, vec!["/p/edited".to_string()]);
    }

    // ── advance_breaker_on_failure — trip-exactly-once semantics ───

    #[test]
    fn test_advance_breaker_first_failure_does_not_trip() {
        let (ledger, newly_tripped) =
            advance_breaker_on_failure(&breaker::FailureLedger::default(), now());
        assert_eq!(ledger.consecutive, 1);
        assert_eq!(ledger.last_failure, Some(now()));
        assert!(!newly_tripped);
    }

    #[test]
    fn test_advance_breaker_threshold_crossing_trips_once() {
        // Walk a clean ledger through THRESHOLD failures: only the
        // crossing failure may report newly_tripped.
        let mut ledger = breaker::FailureLedger::default();
        let mut trips = Vec::new();
        for _ in 0..breaker::THRESHOLD {
            let (next, newly_tripped) = advance_breaker_on_failure(&ledger, now());
            trips.push(newly_tripped);
            ledger = next;
        }
        let expected: Vec<bool> = (1..=breaker::THRESHOLD)
            .map(|n| n == breaker::THRESHOLD)
            .collect();
        assert_eq!(trips, expected, "only the crossing failure trips");
        assert_eq!(ledger.consecutive, breaker::THRESHOLD);
    }

    #[test]
    fn test_advance_breaker_past_threshold_does_not_retrip() {
        let tripped = breaker::FailureLedger {
            consecutive: breaker::THRESHOLD,
            last_failure: Some(now()),
        };
        let (ledger, newly_tripped) = advance_breaker_on_failure(&tripped, now());
        assert_eq!(ledger.consecutive, breaker::THRESHOLD + 1);
        assert!(!newly_tripped, "trip event must not re-fire");
    }

    // ── corruption_notice — fail-open recovery surfacing ───────────

    #[test]
    fn test_corruption_notice_silent_when_nothing_happened() {
        assert!(corruption_notice(false, false, &[]).is_none());
        assert!(
            corruption_notice(false, false, &["/p/a".to_string()]).is_none(),
            "elevated projects alone (no corruption evidence) must not notify"
        );
    }

    #[test]
    fn test_corruption_notice_fresh_recovery_fires_even_without_elevated_projects() {
        let (title, body) = corruption_notice(true, false, &[]).unwrap();
        assert!(title.contains("unreadable"), "title={title}");
        assert!(body.contains("No project currently holds"), "body={body}");
    }

    #[test]
    fn test_corruption_notice_fresh_recovery_lists_elevated_projects() {
        let elevated = vec!["/p/a".to_string(), "/p/b".to_string()];
        let (_, body) = corruption_notice(true, false, &elevated).unwrap();
        assert!(body.contains("/p/a"), "body={body}");
        assert!(body.contains("/p/b"), "body={body}");
        assert!(body.contains("will NOT auto-revert"), "body={body}");
    }

    #[test]
    fn test_corruption_notice_truncates_project_list_to_three() {
        let elevated: Vec<String> = (0..5).map(|i| format!("/p/proj-{i}")).collect();
        let (_, body) = corruption_notice(true, false, &elevated).unwrap();
        assert!(body.contains("/p/proj-0"));
        assert!(body.contains("/p/proj-2"));
        assert!(!body.contains("/p/proj-3"), "body={body}");
        assert!(body.contains("and 2 more"), "body={body}");
    }

    #[test]
    fn test_corruption_notice_stale_copies_fire_only_with_elevated_projects() {
        // Cross-restart detection: forensic copies linger on disk
        // forever, so they only warrant a notification while an
        // unmanaged elevation still exists.
        assert!(corruption_notice(false, true, &[]).is_none());
        let (_, body) = corruption_notice(false, true, &["/p/a".to_string()]).unwrap();
        assert!(body.contains("/p/a"), "body={body}");
    }

    // ── filter_elevated_unmanaged — who gets named in the notice ───

    #[test]
    fn test_filter_elevated_unmanaged_keeps_bypass_without_grant() {
        let file = grants_file(vec![]);
        let modes = vec![
            ("/p/a".to_string(), Some(PermissionMode::BypassPermissions)),
            ("/p/b".to_string(), Some(PermissionMode::Default)),
            ("/p/c".to_string(), None),
        ];
        assert_eq!(
            filter_elevated_unmanaged(modes, &file),
            vec!["/p/a".to_string()]
        );
    }

    #[test]
    fn test_filter_elevated_unmanaged_excludes_regranted_project() {
        // A project the user already re-granted after the recovery is
        // managed again — naming it would be a false alarm.
        let file = grants_file(vec![grant("/p/a", PermissionMode::BypassPermissions, None)]);
        let modes = vec![
            ("/p/a".to_string(), Some(PermissionMode::BypassPermissions)),
            ("/p/b".to_string(), Some(PermissionMode::BypassPermissions)),
        ];
        assert_eq!(
            filter_elevated_unmanaged(modes, &file),
            vec!["/p/b".to_string()]
        );
    }
}
