//! Inline test module for `project_repair.rs`. Lives in this sibling file
//! so `project_repair.rs` stays under the loc-guardian limit; included via
//! `#[cfg(test)] #[path = "project_repair_tests.rs"] mod tests;` so tests
//! still resolve `super::*` against the parent module's internals.

use super::*;
use crate::project_journal::JournalFlags;
use std::time::Duration;
use tempfile::TempDir;

fn write_journal(dir: &Path, id: &str, started_unix: u64) -> PathBuf {
    let j = Journal {
        version: 1,
        started_at: "2026-04-15T00:00:00Z".to_string(),
        started_unix_secs: started_unix,
        pid: 12345,
        hostname: "test-host".to_string(),
        claudepot_version: "0.1.0".to_string(),
        old_path: "/tmp/old".to_string(),
        new_path: "/tmp/new".to_string(),
        old_san: "-tmp-old".to_string(),
        new_san: "-tmp-new".to_string(),
        old_git_root: None,
        new_git_root: None,
        flags: JournalFlags::default(),
        phases_completed: vec!["P3".to_string()],
        snapshot_paths: vec![],
        last_error: None,
    };
    let path = dir.join(format!("{id}.json"));
    fs::write(&path, serde_json::to_string_pretty(&j).unwrap()).unwrap();
    path
}

#[test]
fn test_list_pending_with_status_classifies_pending_vs_stale() {
    let tmp = TempDir::new().unwrap();
    let journals = tmp.path().join("journals");
    let locks = tmp.path().join("locks");
    fs::create_dir_all(&journals).unwrap();
    fs::create_dir_all(&locks).unwrap();

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    write_journal(&journals, "move-fresh", now - 60);
    write_journal(&journals, "move-old", now - 3 * 86_400);

    let entries = list_pending_with_status(&journals, &locks, 86_400).unwrap();
    assert_eq!(entries.len(), 2);
    let fresh = entries.iter().find(|e| e.id == "move-fresh").unwrap();
    let old = entries.iter().find(|e| e.id == "move-old").unwrap();
    assert_eq!(fresh.status, JournalStatus::Pending);
    assert_eq!(old.status, JournalStatus::Stale);
}

#[test]
fn test_list_actionable_excludes_abandoned() {
    let tmp = TempDir::new().unwrap();
    let journals = tmp.path().join("journals");
    let locks = tmp.path().join("locks");
    fs::create_dir_all(&journals).unwrap();
    fs::create_dir_all(&locks).unwrap();

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let a = write_journal(&journals, "move-a", now - 60);
    let _b = write_journal(&journals, "move-b", now - 60);
    project_journal::mark_abandoned(&a).unwrap();

    let actionable = list_actionable(&journals, &locks, 86_400).unwrap();
    assert_eq!(actionable.len(), 1);
    assert_eq!(actionable[0].id, "move-b");
}

/// Variant of `write_journal` that lets the caller set `old_path`.
/// The fixed-`old_path` helper above is enough for status tests
/// but not for `newest_pending_for_old_path`, which keys off the
/// journal's `old_path` field.
fn write_journal_with_old_path(dir: &Path, id: &str, started_unix: u64, old_path: &str) -> PathBuf {
    let j = Journal {
        version: 1,
        started_at: "2026-04-15T00:00:00Z".to_string(),
        started_unix_secs: started_unix,
        pid: 12345,
        hostname: "test-host".to_string(),
        claudepot_version: "0.1.0".to_string(),
        old_path: old_path.to_string(),
        new_path: "/tmp/new".to_string(),
        old_san: project::sanitize_path(old_path),
        new_san: "-tmp-new".to_string(),
        old_git_root: None,
        new_git_root: None,
        flags: JournalFlags::default(),
        phases_completed: vec!["P3".to_string()],
        snapshot_paths: vec![],
        last_error: None,
    };
    let path = dir.join(format!("{id}.json"));
    fs::write(&path, serde_json::to_string_pretty(&j).unwrap()).unwrap();
    path
}

#[test]
fn test_find_pending_by_id_returns_none_when_missing() {
    let tmp = TempDir::new().unwrap();
    let journals = tmp.path().join("journals");
    let locks = tmp.path().join("locks");
    fs::create_dir_all(&journals).unwrap();
    fs::create_dir_all(&locks).unwrap();

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    write_journal(&journals, "move-a", now - 60);

    let got = find_pending_by_id(&journals, &locks, 86_400, "move-does-not-exist").unwrap();
    assert!(got.is_none());
}

#[test]
fn test_find_pending_by_id_returns_entry_when_present() {
    let tmp = TempDir::new().unwrap();
    let journals = tmp.path().join("journals");
    let locks = tmp.path().join("locks");
    fs::create_dir_all(&journals).unwrap();
    fs::create_dir_all(&locks).unwrap();

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    write_journal(&journals, "move-a", now - 60);
    write_journal(&journals, "move-b", now - 120);

    let got = find_pending_by_id(&journals, &locks, 86_400, "move-b")
        .unwrap()
        .expect("move-b should resolve");
    assert_eq!(got.id, "move-b");
    assert_eq!(got.journal.started_unix_secs, now - 120);
}

#[test]
fn test_newest_pending_for_old_path_picks_max_started_unix_secs() {
    let tmp = TempDir::new().unwrap();
    let journals = tmp.path().join("journals");
    let locks = tmp.path().join("locks");
    fs::create_dir_all(&journals).unwrap();
    fs::create_dir_all(&locks).unwrap();

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    // Three journals share /tmp/old; one points at a different path.
    write_journal_with_old_path(&journals, "move-old-1", now - 300, "/tmp/old");
    write_journal_with_old_path(&journals, "move-old-2", now - 100, "/tmp/old");
    write_journal_with_old_path(&journals, "move-old-3", now - 200, "/tmp/old");
    write_journal_with_old_path(&journals, "move-other", now - 50, "/tmp/other");

    let got = newest_pending_for_old_path(&journals, &locks, 86_400, "/tmp/old")
        .unwrap()
        .expect("a /tmp/old journal should win");
    assert_eq!(got.id, "move-old-2");
    assert_eq!(got.journal.started_unix_secs, now - 100);
}

#[test]
fn test_newest_pending_for_old_path_returns_none_when_no_match() {
    let tmp = TempDir::new().unwrap();
    let journals = tmp.path().join("journals");
    let locks = tmp.path().join("locks");
    fs::create_dir_all(&journals).unwrap();
    fs::create_dir_all(&locks).unwrap();

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    write_journal_with_old_path(&journals, "move-a", now - 60, "/tmp/somewhere");

    let got = newest_pending_for_old_path(&journals, &locks, 86_400, "/tmp/nothing-here").unwrap();
    assert!(got.is_none());
}

#[test]
fn test_abandon_writes_sidecar_and_preserves_journal() {
    let tmp = TempDir::new().unwrap();
    let journals = tmp.path().join("journals");
    let locks = tmp.path().join("locks");
    fs::create_dir_all(&journals).unwrap();
    fs::create_dir_all(&locks).unwrap();

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    write_journal(&journals, "move-x", now - 60);

    let entries = list_actionable(&journals, &locks, 86_400).unwrap();
    let sidecar = abandon(&entries[0]).unwrap();
    assert!(sidecar.exists());
    assert!(entries[0].path.exists(), "journal is preserved for audit");

    // Second pass sees no actionable entries.
    let after = list_actionable(&journals, &locks, 86_400).unwrap();
    assert!(after.is_empty());
}

#[test]
fn test_break_lock_with_audit_writes_audit_record() {
    let tmp = TempDir::new().unwrap();
    let journals = tmp.path().join("journals");
    let locks = tmp.path().join("locks");
    fs::create_dir_all(&journals).unwrap();
    fs::create_dir_all(&locks).unwrap();

    // Fabricate a lock file directly.
    let lock_path = locks.join("-tmp-foo.lock");
    let lock = Lock {
        version: 1,
        pid: 99999,
        hostname: "ghost-host".to_string(),
        start_iso8601: "2026-04-15T00:00:00Z".to_string(),
        start_unix_secs: 0,
        claudepot_version: "0.1.0".to_string(),
    };
    fs::write(&lock_path, serde_json::to_string(&lock).unwrap()).unwrap();

    let broken = break_lock_with_audit(&lock_path, &journals).unwrap();
    assert!(!lock_path.exists(), "lock removed");
    assert!(broken.audit_path.exists(), "audit written");
    assert_eq!(broken.prior.pid, 99999);

    let audit = fs::read_to_string(&broken.audit_path).unwrap();
    assert!(audit.contains("\"reason\""));
    assert!(audit.contains("manual --break-lock"));
}

#[test]
fn test_gc_dry_run_does_not_delete() {
    let tmp = TempDir::new().unwrap();
    let journals = tmp.path().join("journals");
    let snaps = tmp.path().join("snapshots");
    fs::create_dir_all(&journals).unwrap();
    fs::create_dir_all(&snaps).unwrap();

    // Seed an abandoned journal with an old mtime.
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let journal_path = write_journal(&journals, "move-old", now - 100 * 86_400);
    project_journal::mark_abandoned(&journal_path).unwrap();
    // Backdate sidecar mtime by 100 days.
    let sidecar = journals.join("move-old.abandoned.json");
    let old_time = SystemTime::now()
        .checked_sub(Duration::from_secs(100 * 86_400))
        .unwrap();
    filetime::set_file_mtime(&sidecar, old_time.into()).ok();
    filetime::set_file_mtime(&journal_path, old_time.into()).ok();

    let result = gc(&journals, &snaps, 30, /* dry_run */ true).unwrap();
    assert_eq!(result.removed_journals, 0);
    assert!(!result.would_remove.is_empty());
    assert!(sidecar.exists(), "dry-run preserves files");
    assert!(journal_path.exists());
}

#[test]
fn test_gc_removes_abandoned_journals_older_than_cutoff() {
    let tmp = TempDir::new().unwrap();
    let journals = tmp.path().join("journals");
    let snaps = tmp.path().join("snapshots");
    fs::create_dir_all(&journals).unwrap();
    fs::create_dir_all(&snaps).unwrap();

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let journal_path = write_journal(&journals, "move-old", now - 100 * 86_400);
    project_journal::mark_abandoned(&journal_path).unwrap();
    let sidecar = journals.join("move-old.abandoned.json");
    let old_time = SystemTime::now()
        .checked_sub(Duration::from_secs(100 * 86_400))
        .unwrap();
    filetime::set_file_mtime(&sidecar, old_time.into()).ok();
    filetime::set_file_mtime(&journal_path, old_time.into()).ok();

    let result = gc(&journals, &snaps, 30, /* dry_run */ false).unwrap();
    assert_eq!(result.removed_journals, 1);
    assert!(!sidecar.exists());
    assert!(!journal_path.exists());
    assert!(result.bytes_freed > 0);
}

#[test]
fn test_gc_leaves_recent_journals_alone() {
    let tmp = TempDir::new().unwrap();
    let journals = tmp.path().join("journals");
    let snaps = tmp.path().join("snapshots");
    fs::create_dir_all(&journals).unwrap();
    fs::create_dir_all(&snaps).unwrap();

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let journal_path = write_journal(&journals, "move-new", now - 60);
    project_journal::mark_abandoned(&journal_path).unwrap();

    let result = gc(&journals, &snaps, 30, /* dry_run */ false).unwrap();
    assert_eq!(result.removed_journals, 0);
    assert!(journal_path.exists());
}

#[test]
fn test_resolve_lock_file_by_sanitized_path() {
    let tmp = TempDir::new().unwrap();
    let locks = tmp.path().join("locks");
    fs::create_dir_all(&locks).unwrap();
    let san = project::sanitize_path("/tmp/my-project");
    let lock_path = locks.join(format!("{san}.lock"));
    fs::write(&lock_path, "{}").unwrap();

    let resolved = resolve_lock_file(&locks, "/tmp/my-project").unwrap();
    assert_eq!(resolved, lock_path);
}

#[test]
fn test_resolve_lock_file_by_bare_sanitized_name() {
    let tmp = TempDir::new().unwrap();
    let locks = tmp.path().join("locks");
    fs::create_dir_all(&locks).unwrap();
    let lock_path = locks.join("-tmp-bare.lock");
    fs::write(&lock_path, "{}").unwrap();

    let resolved = resolve_lock_file(&locks, "-tmp-bare").unwrap();
    assert_eq!(resolved, lock_path);
}

#[test]
fn test_resolve_lock_file_missing_returns_none() {
    let tmp = TempDir::new().unwrap();
    let locks = tmp.path().join("locks");
    fs::create_dir_all(&locks).unwrap();
    assert!(resolve_lock_file(&locks, "/nowhere").is_none());
}

// -----------------------------------------------------------------
// cleanup_abandoned / preview_abandoned
// -----------------------------------------------------------------

fn write_journal_with_snapshots(journals_dir: &Path, id: &str, snapshots: &[PathBuf]) -> PathBuf {
    let j = Journal {
        version: 1,
        started_at: "2026-04-15T00:00:00Z".to_string(),
        started_unix_secs: 1_700_000_000,
        pid: 12345,
        hostname: "test-host".to_string(),
        claudepot_version: "0.1.0".to_string(),
        old_path: "/tmp/old".to_string(),
        new_path: "/tmp/new".to_string(),
        old_san: "-tmp-old".to_string(),
        new_san: "-tmp-new".to_string(),
        old_git_root: None,
        new_git_root: None,
        flags: JournalFlags::default(),
        phases_completed: vec!["P3".to_string()],
        snapshot_paths: snapshots.to_vec(),
        last_error: None,
    };
    let path = journals_dir.join(format!("{id}.json"));
    fs::write(&path, serde_json::to_string_pretty(&j).unwrap()).unwrap();
    path
}

fn write_sidecar(journals_dir: &Path, id: &str) {
    fs::write(journals_dir.join(format!("{id}.abandoned.json")), "{}").unwrap();
}

#[test]
fn preview_abandoned_reports_journal_sidecar_and_referenced_snapshots() {
    let tmp = TempDir::new().unwrap();
    let journals = tmp.path().join("journals");
    let snapshots = tmp.path().join("snapshots");
    fs::create_dir_all(&journals).unwrap();
    fs::create_dir_all(&snapshots).unwrap();

    let snap_a = snapshots.join("ts-1-P7.json");
    let snap_b = snapshots.join("ts-1-P8.json");
    fs::write(&snap_a, "a").unwrap(); // 1 byte
    fs::write(&snap_b, "bb").unwrap(); // 2 bytes

    let journal_path = write_journal_with_snapshots(
        &journals,
        "move-abandoned",
        &[snap_a.clone(), snap_b.clone()],
    );
    write_sidecar(&journals, "move-abandoned");

    let report = preview_abandoned(&journals).expect("preview");
    assert_eq!(report.entries.len(), 1);
    let entry = &report.entries[0];
    assert_eq!(entry.id, "move-abandoned");
    assert_eq!(entry.journal_path, journal_path);
    assert_eq!(
        entry.referenced_snapshots,
        vec![snap_a.clone(), snap_b.clone()]
    );
    assert!(
        entry.bytes >= 3,
        "bytes must account for snapshots at minimum"
    );
    // Preview must NOT delete anything.
    assert!(snap_a.exists());
    assert!(snap_b.exists());
    assert!(journal_path.exists());
}

#[test]
fn cleanup_abandoned_removes_journal_sidecar_and_referenced_snapshots() {
    let tmp = TempDir::new().unwrap();
    let journals = tmp.path().join("journals");
    let snapshots = tmp.path().join("snapshots");
    fs::create_dir_all(&journals).unwrap();
    fs::create_dir_all(&snapshots).unwrap();

    let snap = snapshots.join("ts-1-P7.json");
    fs::write(&snap, "bytes").unwrap();
    let journal_path = write_journal_with_snapshots(&journals, "move-abandoned", &[snap.clone()]);
    let sidecar_path = journals.join("move-abandoned.abandoned.json");
    write_sidecar(&journals, "move-abandoned");

    let report = cleanup_abandoned(&journals).expect("cleanup");
    assert_eq!(report.removed_journals, 1);
    assert_eq!(report.removed_snapshots, 1);
    assert_eq!(report.entries.len(), 1);
    assert!(report.bytes_freed >= 5);
    assert!(!journal_path.exists(), "journal must be removed");
    assert!(!sidecar_path.exists(), "sidecar must be removed");
    assert!(!snap.exists(), "referenced snapshot must be removed");
}

#[test]
fn cleanup_abandoned_leaves_unreferenced_and_non_abandoned_artifacts_alone() {
    // This is the load-bearing safety test: gc(0, ...) would sweep
    // these too. cleanup_abandoned MUST NOT.
    let tmp = TempDir::new().unwrap();
    let journals = tmp.path().join("journals");
    let snapshots = tmp.path().join("snapshots");
    fs::create_dir_all(&journals).unwrap();
    fs::create_dir_all(&snapshots).unwrap();

    // A live journal (no sidecar).
    let live_journal = write_journal_with_snapshots(&journals, "move-live", &[]);
    // A snapshot that isn't referenced by any journal — e.g. from
    // a successful rename. Must survive cleanup.
    let orphan_snap = snapshots.join("ts-99-P7.json");
    fs::write(&orphan_snap, "orphan").unwrap();
    // An abandoned journal with its own referenced snapshot.
    let referenced = snapshots.join("ts-1-P7.json");
    fs::write(&referenced, "x").unwrap();
    let abandoned_journal =
        write_journal_with_snapshots(&journals, "move-abandoned", &[referenced.clone()]);
    write_sidecar(&journals, "move-abandoned");

    let report = cleanup_abandoned(&journals).expect("cleanup");
    assert_eq!(report.removed_journals, 1);
    assert_eq!(report.removed_snapshots, 1);

    // Live journal untouched.
    assert!(live_journal.exists(), "live journal must survive");
    // Orphan snapshot untouched — this is the difference from gc.
    assert!(
        orphan_snap.exists(),
        "unreferenced snapshot must survive cleanup_abandoned"
    );
    // Abandoned artifacts gone.
    assert!(!abandoned_journal.exists());
    assert!(!referenced.exists());
}

#[test]
fn cleanup_abandoned_returns_empty_when_no_sidecars_exist() {
    let tmp = TempDir::new().unwrap();
    let journals = tmp.path().join("journals");
    fs::create_dir_all(&journals).unwrap();
    let _ = write_journal_with_snapshots(&journals, "move-live", &[]);

    let report = cleanup_abandoned(&journals).expect("cleanup");
    assert!(report.entries.is_empty());
    assert_eq!(report.removed_journals, 0);
    assert_eq!(report.removed_snapshots, 0);
}

#[test]
fn cleanup_abandoned_tolerates_missing_snapshot_paths() {
    // If a snapshot listed in snapshot_paths was already removed
    // manually (user ran `rm`), cleanup_abandoned should still
    // succeed and remove the journal + sidecar without counting
    // the missing snapshot.
    let tmp = TempDir::new().unwrap();
    let journals = tmp.path().join("journals");
    fs::create_dir_all(&journals).unwrap();

    let phantom = tmp.path().join("this-was-deleted-out-of-band.json");
    let journal_path = write_journal_with_snapshots(&journals, "move-abandoned", &[phantom]);
    write_sidecar(&journals, "move-abandoned");

    let report = cleanup_abandoned(&journals).expect("cleanup");
    assert_eq!(report.removed_journals, 1);
    assert_eq!(report.removed_snapshots, 0);
    assert!(!journal_path.exists());
}
