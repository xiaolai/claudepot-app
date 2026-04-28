//! Inline test module for `index.rs`. Lives in this sibling file
//! so `index.rs` stays under the loc-guardian limit; included via
//! `#[cfg(test)] #[path = "index_tests.rs"] mod tests;` so tests
//! still resolve `super::*` against the parent module's internals.

use super::*;
use crate::activity::card::CardKind;
use chrono::Utc;
use std::collections::BTreeMap;
use tempfile::tempdir;

fn sample_card(uuid: &str, byte: u64, title: &str) -> Card {
    let mut args = BTreeMap::new();
    args.insert("plugin".to_string(), "mermaid-preview@xiaolai".to_string());
    Card {
        id: None,
        session_path: PathBuf::from("/tmp/x.jsonl"),
        event_uuid: Some(uuid.into()),
        byte_offset: byte,
        kind: CardKind::HookFailure,
        ts: Utc::now(),
        severity: Severity::Warn,
        title: title.into(),
        subtitle: Some("bash failed".into()),
        help: Some(HelpRef {
            template_id: "hook.plugin_missing".into(),
            args,
        }),
        source_ref: None,
        cwd: PathBuf::from("/Users/x/proj"),
        git_branch: Some("main".into()),
        plugin: None,
    }
}

#[test]
fn open_creates_table_and_starts_empty() {
    let dir = tempdir().unwrap();
    let idx = ActivityIndex::open(&dir.path().join("a.db")).unwrap();
    assert_eq!(idx.row_count().unwrap(), 0);
}

/// Regression for Codex audit MEDIUM #6 / LOW #12: `open()` must
/// quarantine a corrupt DB and rebuild from scratch, mirroring
/// `SessionIndex`. The activity index is a derivation, so wipe-
/// and-rebuild is always safe — we don't want a corrupted file
/// to brick `claudepot activity` commands.
#[test]
fn open_quarantines_corrupt_db_and_rebuilds() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("a.db");
    // Plant a non-SQLite file at the target path. SQLite will
    // refuse it with NotADatabase; the open() retry path must
    // rename it aside and create a fresh DB.
    std::fs::write(&path, b"not a sqlite database, just bytes").unwrap();
    let idx = ActivityIndex::open(&path).unwrap();
    assert_eq!(idx.row_count().unwrap(), 0, "fresh DB must start empty");

    let sibs: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        sibs.iter().any(|n| n.contains(".db.corrupt-")),
        "quarantined file must remain on disk: {sibs:?}"
    );
}

/// Regression for Codex audit LOW #12 (sidecar perms): the
/// touch-write trick in `init_connection` must materialize the
/// WAL/SHM sidecars on first open so the chmod loop can narrow
/// their perms. Without it, the first write to the DB would
/// create them with the process umask (typically 0644).
#[cfg(unix)]
#[test]
fn open_narrows_wal_shm_sidecar_perms() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempdir().unwrap();
    let path = dir.path().join("a.db");
    let _idx = ActivityIndex::open(&path).unwrap();
    for ext in ["db-wal", "db-shm"] {
        let side = path.with_extension(ext);
        assert!(side.exists(), "{ext} sidecar must exist after open");
        let mode = std::fs::metadata(&side).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "{ext} perms should be 0o600, got {mode:o}");
    }
}

/// Regression for Codex audit MEDIUM #7: row decode must reject
/// rows with unknown enum labels rather than fabricate `Info`/
/// `ToolError` defaults. The bad row is logged and skipped; the
/// healthy rows still come back.
#[test]
fn recent_skips_rows_with_unknown_kind_label() {
    let dir = tempdir().unwrap();
    let idx = ActivityIndex::open(&dir.path().join("a.db")).unwrap();
    idx.insert(&sample_card("u1", 0, "good")).unwrap();

    // Inject a row with a bogus kind label — bypass the public
    // insert path so we can simulate a forward-version row.
    {
        let db = idx.db();
        db.execute(
            "INSERT INTO activity_cards (session_path, event_uuid, byte_offset, kind, severity, ts_ms, title, cwd) \
             VALUES ('/tmp/bad.jsonl', 'u-bad', 0, 'kind-from-future-version', 'WARN', 0, 'bad', '/x')",
            [],
        )
        .unwrap();
    }

    let cards = idx.recent(&RecentQuery::default()).unwrap();
    assert_eq!(cards.len(), 1, "bad row skipped, good row returned");
    assert_eq!(cards[0].title, "good");
}

/// Phase 2: `last_seen_card_id` cursor round-trips. Fresh DB →
/// `None`. After `set_last_seen(N)` → `Some(N)`. Re-setting is
/// an UPSERT, not an error.
#[test]
fn last_seen_cursor_round_trips() {
    let dir = tempdir().unwrap();
    let idx = ActivityIndex::open(&dir.path().join("a.db")).unwrap();
    assert_eq!(idx.last_seen().unwrap(), None);

    idx.set_last_seen(42).unwrap();
    assert_eq!(idx.last_seen().unwrap(), Some(42));

    // UPSERT — re-setting overwrites without error.
    idx.set_last_seen(43).unwrap();
    assert_eq!(idx.last_seen().unwrap(), Some(43));
}

/// Phase 2: `count_new_since` returns rows above the cursor that
/// also match the filter set. Drives the "N new since you were
/// away" badge.
#[test]
fn count_new_since_respects_cursor_and_filters() {
    let dir = tempdir().unwrap();
    let idx = ActivityIndex::open(&dir.path().join("a.db")).unwrap();
    let mut warn = sample_card("u1", 0, "warn-card");
    warn.severity = Severity::Warn;
    let mut info = sample_card("u2", 1, "info-card");
    info.kind = CardKind::ToolError;
    info.severity = Severity::Info;
    let id_warn = idx.insert(&warn).unwrap().unwrap();
    let id_info = idx.insert(&info).unwrap().unwrap();
    assert!(id_warn < id_info, "rowid is monotonic");

    // No cursor: every row counts.
    let total = idx.count_new_since(None, &RecentQuery::default()).unwrap();
    assert_eq!(total, 2);

    // Cursor at id_warn: only the info row is "new."
    let after_warn = idx
        .count_new_since(Some(id_warn), &RecentQuery::default())
        .unwrap();
    assert_eq!(after_warn, 1);

    // Cursor + severity filter: cursor excludes warn, filter
    // would exclude info, so net zero.
    let new_warns_after = idx
        .count_new_since(
            Some(id_warn),
            &RecentQuery {
                min_severity: Some(Severity::Warn),
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(new_warns_after, 0);

    // Plugin filter parity with recent(): inserting a card with
    // a plugin attribution and counting "new since 0" with that
    // filter must agree with recent() (1, not 0).
    let mut tagged = sample_card("u3", 2, "tagged");
    tagged.plugin = Some("mermaid-preview@xiaolai".to_string());
    let id_tagged = idx.insert(&tagged).unwrap().unwrap();
    let only_plugin = RecentQuery {
        plugin: Some("mermaid-preview".to_string()),
        ..Default::default()
    };
    assert_eq!(idx.recent(&only_plugin).unwrap().len(), 1);
    assert_eq!(
        idx.count_new_since(Some(id_tagged - 1), &only_plugin)
            .unwrap(),
        1,
        "count_new_since must honor plugin filter — parity with recent()"
    );
}

/// Regression for Codex audit LOW #3: project-prefix filtering
/// must NOT treat `%` and `_` in the user's path as SQL wildcards.
/// A path containing `%` should match exactly, not as a glob.
#[test]
fn project_filter_does_not_treat_percent_as_wildcard() {
    let dir = tempdir().unwrap();
    let idx = ActivityIndex::open(&dir.path().join("a.db")).unwrap();
    let mut a = sample_card("u-a", 0, "a");
    a.cwd = PathBuf::from("/Users/x/proj");
    let mut b = sample_card("u-b", 0, "b");
    b.session_path = PathBuf::from("/tmp/y.jsonl");
    b.cwd = PathBuf::from("/Users/x/other");
    idx.insert(&a).unwrap();
    idx.insert(&b).unwrap();

    // Filtering by "/Users/x/p%" used to match BOTH rows under
    // a LIKE-based filter (because % is a wildcard there); the
    // substr-based filter must match neither.
    let q = RecentQuery {
        project_path_prefix: Some(PathBuf::from("/Users/x/p%")),
        ..Default::default()
    };
    let cards = idx.recent(&q).unwrap();
    assert!(
        cards.is_empty(),
        "literal % in prefix must not glob: got {:?}",
        cards.iter().map(|c| c.title.clone()).collect::<Vec<_>>()
    );

    // Sanity: a real prefix still filters correctly.
    let q2 = RecentQuery {
        project_path_prefix: Some(PathBuf::from("/Users/x/proj")),
        ..Default::default()
    };
    let only_a = idx.recent(&q2).unwrap();
    assert_eq!(only_a.len(), 1);
    assert_eq!(only_a[0].title, "a");
}

#[test]
fn insert_and_recent_round_trip() {
    let dir = tempdir().unwrap();
    let idx = ActivityIndex::open(&dir.path().join("a.db")).unwrap();
    let id = idx
        .insert(&sample_card("u1", 0, "Hook failed: PostToolUse:Edit"))
        .unwrap();
    assert!(id.is_some());
    assert_eq!(idx.row_count().unwrap(), 1);
    let cards = idx.recent(&RecentQuery::default()).unwrap();
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].title, "Hook failed: PostToolUse:Edit");
    assert_eq!(
        cards[0].help.as_ref().unwrap().template_id,
        "hook.plugin_missing"
    );
}

/// The idempotency invariant — re-inserting a card with the same
/// (session_path, event_uuid) is a no-op. Re-running backfill on
/// the same JSONL must NOT duplicate rows.
#[test]
fn re_insert_same_uuid_is_a_no_op() {
    let dir = tempdir().unwrap();
    let idx = ActivityIndex::open(&dir.path().join("a.db")).unwrap();
    let c = sample_card("u1", 0, "first");
    let _ = idx.insert(&c).unwrap();
    let second = idx.insert(&c).unwrap();
    assert!(second.is_none(), "duplicate insert returns None");
    assert_eq!(idx.row_count().unwrap(), 1, "no duplicate row");
}

/// Lines without a uuid fall back to (session_path, byte_offset)
/// uniqueness — same offset twice is a no-op, different offset
/// is a new row.
#[test]
fn null_uuid_dedupes_on_offset() {
    let dir = tempdir().unwrap();
    let idx = ActivityIndex::open(&dir.path().join("a.db")).unwrap();
    let mut c = sample_card("ignored", 100, "no-uuid card");
    c.event_uuid = None;
    let first = idx.insert(&c).unwrap();
    let second = idx.insert(&c).unwrap();
    assert!(first.is_some());
    assert!(second.is_none());
    assert_eq!(idx.row_count().unwrap(), 1);

    let mut c2 = c.clone();
    c2.byte_offset = 200;
    let third = idx.insert(&c2).unwrap();
    assert!(third.is_some(), "different offset → new row");
    assert_eq!(idx.row_count().unwrap(), 2);
}

#[test]
fn delete_for_session_clears_only_that_session() {
    let dir = tempdir().unwrap();
    let idx = ActivityIndex::open(&dir.path().join("a.db")).unwrap();
    let mut c1 = sample_card("u1", 0, "session1");
    c1.session_path = PathBuf::from("/tmp/s1.jsonl");
    let mut c2 = sample_card("u2", 0, "session2");
    c2.session_path = PathBuf::from("/tmp/s2.jsonl");
    idx.insert(&c1).unwrap();
    idx.insert(&c2).unwrap();
    let n = idx.delete_for_session(Path::new("/tmp/s1.jsonl")).unwrap();
    assert_eq!(n, 1);
    assert_eq!(idx.row_count().unwrap(), 1);
    let remaining = idx.recent(&RecentQuery::default()).unwrap();
    assert_eq!(remaining[0].title, "session2");
}

#[test]
fn recent_filters_by_kind_and_severity() {
    let dir = tempdir().unwrap();
    let idx = ActivityIndex::open(&dir.path().join("a.db")).unwrap();
    let mut warn_card = sample_card("u1", 0, "warn");
    warn_card.severity = Severity::Warn;
    let mut error_card = sample_card("u2", 1, "error");
    error_card.kind = CardKind::ToolError;
    error_card.severity = Severity::Error;
    idx.insert(&warn_card).unwrap();
    idx.insert(&error_card).unwrap();

    let only_errors = idx
        .recent(&RecentQuery {
            min_severity: Some(Severity::Error),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(only_errors.len(), 1);
    assert_eq!(only_errors[0].title, "error");

    let only_hooks = idx
        .recent(&RecentQuery {
            kinds: vec![CardKind::HookFailure],
            ..Default::default()
        })
        .unwrap();
    assert_eq!(only_hooks.len(), 1);
    assert_eq!(only_hooks[0].title, "warn");
}

/// Bulk insert keeps the same idempotency contract — re-running
/// the backfill yields zero new rows on the second pass. This is
/// the rebuild safety net.
#[test]
fn insert_many_is_idempotent() {
    let dir = tempdir().unwrap();
    let idx = ActivityIndex::open(&dir.path().join("a.db")).unwrap();
    let cards = vec![
        sample_card("u1", 0, "a"),
        sample_card("u2", 1, "b"),
        sample_card("u3", 2, "c"),
    ];
    let (ins, skipped) = idx.insert_many(&cards).unwrap();
    assert_eq!((ins, skipped), (3, 0));
    let (ins2, skipped2) = idx.insert_many(&cards).unwrap();
    assert_eq!((ins2, skipped2), (0, 3));
    assert_eq!(idx.row_count().unwrap(), 3);
}
