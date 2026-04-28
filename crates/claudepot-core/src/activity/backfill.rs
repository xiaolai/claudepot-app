//! One-shot backfill — walk every JSONL under `<config_dir>/projects/`,
//! classify each line, persist resulting cards. Phase 1 ingest path.
//!
//! Re-running is safe and cheap when the cache is warm: every card
//! insert is idempotent on `(session_path, event_uuid)`, so a second
//! pass over an unchanged JSONL emits zero new rows. A future
//! `(size, mtime_ns)` skip-fast guard (mirroring `SessionIndex`)
//! lands when the per-file cost shows up in real timing — for v1,
//! "scan everything" is fine since the work is bounded by the same
//! filesystem walk `SessionIndex::refresh` already does in seconds.
//!
//! Per-file errors are collected into `BackfillStats.failed` rather
//! than propagated. One unreadable JSONL must never abort the whole
//! backfill — the same posture `session_index::refresh` takes.

use rayon::prelude::*;
use serde_json::Value;
use std::io::{self, BufRead, BufReader};
use std::path::{Path, PathBuf};

use super::card::Card;
use super::classifier::{classify, ClassifierState, SessionMeta};
use super::index::{ActivityIndex, ActivityIndexError};

/// Outcome of a backfill pass. Counts and a `failed` list rather
/// than `Result<()>` so callers can surface partial degradation —
/// the index is still useful even if 1% of files were unreadable.
#[derive(Debug, Default)]
pub struct BackfillStats {
    pub files_scanned: usize,
    pub cards_inserted: usize,
    pub cards_skipped_duplicates: usize,
    /// Cards that existed in the index from prior runs but whose
    /// source JSONL no longer exists (typically: the user moved or
    /// deleted a session via `claudepot session move`/trash). Cleared
    /// from the table — these counts let the CLI report the rebuild
    /// effect. Always 0 in incremental ingest paths that don't sweep.
    pub cards_pruned: usize,
    pub failed: Vec<(PathBuf, String)>,
    pub elapsed: std::time::Duration,
}

/// Walk `<config_dir>/projects/*/*.jsonl`, classify, persist. The
/// per-file work happens on the rayon thread pool; SQLite writes
/// serialize through the index's mutex.
///
/// Sweeps stale rows out of the index too: any `session_path` in the
/// `activity_cards` table that no longer exists on disk has its rows
/// dropped before the (re)insertion pass. This makes `reindex` a
/// genuine rebuild rather than an additive accumulator — fixes the
/// case where a deleted transcript leaves orphaned cards behind.
/// Live JSONLs are also dropped-then-replayed so a transcript that
/// was edited (e.g. via `session slim`) gets a fresh classification
/// instead of stacking new cards on top of stale ones.
pub fn run(config_dir: &Path, idx: &ActivityIndex) -> Result<BackfillStats, ActivityIndexError> {
    let started = std::time::Instant::now();
    let mut stats = BackfillStats::default();
    let projects_dir = config_dir.join("projects");
    if !projects_dir.exists() {
        // The whole tree is gone — sweep every row in the index.
        // Otherwise a user who deletes `~/.claude/projects/` and
        // reruns reindex would see stale cards forever. The empty
        // live-set passed to `session_paths_not_in` returns ALL
        // distinct session_paths in the index.
        let stale = idx.session_paths_not_in(&Default::default())?;
        for stale_path in &stale {
            stats.cards_pruned += idx.delete_for_session(stale_path)?;
        }
        stats.elapsed = started.elapsed();
        return Ok(stats);
    }

    // Walk in two steps: collect every JSONL path, then map in
    // parallel. Per-entry walk errors land in `stats.failed` instead
    // of aborting the backfill (matches `session_index::refresh`
    // posture — one unreadable slug must not blow away the index).
    let files = collect_jsonl_paths(&projects_dir, &mut stats.failed);

    // Sweep stale: drop any cards whose source JSONL no longer
    // exists. Done before the per-file replay so the second pass's
    // delete-and-replay (below) doesn't have to consider them.
    let live_paths: std::collections::HashSet<PathBuf> = files.iter().cloned().collect();
    let stale = idx.session_paths_not_in(&live_paths)?;
    for stale_path in &stale {
        let n = idx.delete_for_session(stale_path)?;
        stats.cards_pruned += n;
    }

    // Classify each live file in parallel — pure CPU + read I/O.
    // Bulk SQLite writes happen on the calling thread to keep the
    // lock window tight.
    let per_file: Vec<(PathBuf, Result<Vec<Card>, String>)> = files
        .par_iter()
        .map(|path| {
            let result = classify_file(path).map_err(|e| e.to_string());
            (path.clone(), result)
        })
        .collect();

    // Per-session rebuild: drop the existing rows for each replayed
    // file before re-inserting. Without this, an edited JSONL would
    // stack new cards on top of the previous run's rows, growing the
    // table monotonically. With it, the table converges to the
    // current JSONL state on every full pass.
    let mut all_cards: Vec<Card> = Vec::new();
    for (path, res) in per_file {
        stats.files_scanned += 1;
        match res {
            Ok(cards) => {
                idx.delete_for_session(&path)?;
                all_cards.extend(cards);
            }
            Err(e) => stats.failed.push((path, e)),
        }
    }

    let (inserted, skipped) = idx.insert_many(&all_cards)?;
    stats.cards_inserted = inserted;
    stats.cards_skipped_duplicates = skipped;
    stats.elapsed = started.elapsed();
    Ok(stats)
}

/// Read one JSONL, run `classify` line-by-line, return all emitted
/// cards. Each line's byte offset is the position of its first
/// byte in the file — what `body_at` will use to seek lazily.
///
/// Empty lines and lines that fail to parse as JSON are skipped
/// silently (matches `parse_line_into`'s tolerance).
pub fn classify_file(path: &Path) -> io::Result<Vec<Card>> {
    let file = std::fs::File::open(path)?;
    let mut reader = BufReader::new(file);
    let mut state = ClassifierState::default();
    let session_path = path.to_path_buf();
    let mut meta = SessionMeta {
        session_path: session_path.clone(),
        cwd: PathBuf::new(),
        git_branch: None,
    };

    let mut cards: Vec<Card> = Vec::new();
    let mut byte_offset: u64 = 0;
    let mut buf = String::new();
    loop {
        buf.clear();
        let read = reader.read_line(&mut buf)?;
        if read == 0 {
            break;
        }
        let trimmed = buf.trim_end_matches(['\n', '\r']);
        if trimmed.is_empty() {
            byte_offset += read as u64;
            continue;
        }

        if let Ok(v) = serde_json::from_str::<Value>(trimmed) {
            // Refresh session-level meta opportunistically — the
            // classifier prefers per-line cwd/gitBranch when present
            // (CC writes them on every record), but having a
            // last-seen fallback keeps cards complete even if a
            // record happens to omit one.
            if let Some(c) = v.get("cwd").and_then(Value::as_str) {
                meta.cwd = PathBuf::from(c);
            }
            if let Some(b) = v.get("gitBranch").and_then(Value::as_str) {
                meta.git_branch = Some(b.to_string());
            }
            cards.extend(classify(&v, byte_offset, &meta, &mut state));
        }
        byte_offset += read as u64;
    }
    Ok(cards)
}

/// Walk `<projects_dir>/<slug>/*.jsonl`. Per-entry walk failures
/// (transient ENOENT, EACCES on a single slug, file-type race) push
/// into `failed` rather than aborting the whole walk. Mirrors
/// `session_index::codec::walk_fs`'s posture: a partial result is
/// always more useful than no result.
fn collect_jsonl_paths(projects_dir: &Path, failed: &mut Vec<(PathBuf, String)>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let top = match std::fs::read_dir(projects_dir) {
        Ok(it) => it,
        Err(e) => {
            failed.push((projects_dir.to_path_buf(), e.to_string()));
            return out;
        }
    };
    for slug_res in top {
        let slug_entry = match slug_res {
            Ok(e) => e,
            Err(e) => {
                failed.push((projects_dir.to_path_buf(), e.to_string()));
                continue;
            }
        };
        let ft = match slug_entry.file_type() {
            Ok(ft) => ft,
            Err(e) => {
                failed.push((slug_entry.path(), e.to_string()));
                continue;
            }
        };
        if !ft.is_dir() {
            continue;
        }
        let inner = match std::fs::read_dir(slug_entry.path()) {
            Ok(it) => it,
            Err(e) => {
                failed.push((slug_entry.path(), e.to_string()));
                continue;
            }
        };
        for sess_res in inner {
            let session_entry = match sess_res {
                Ok(e) => e,
                Err(e) => {
                    failed.push((slug_entry.path(), e.to_string()));
                    continue;
                }
            };
            let name = session_entry.file_name();
            let name_str = name.to_string_lossy();
            if name_str.ends_with(".jsonl") {
                out.push(session_entry.path());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn write_jsonl(path: &Path, lines: &[&str]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, lines.join("\n") + "\n").unwrap();
    }

    /// End-to-end Phase 1 demo path: a fixture JSONL with a known
    /// plugin_missing failure flows through the classifier and
    /// lands in SQLite with the expected help template id.
    #[test]
    fn run_classifies_real_fixture_into_index() {
        let dir = tempdir().unwrap();
        let config = dir.path().join("claude");
        let project = config.join("projects").join("-Users-x-proj");
        let jsonl_path = project.join("sess.jsonl");
        let fixture = include_str!("testdata/hook_plugin_missing.jsonl").trim();
        write_jsonl(&jsonl_path, &[fixture]);

        let db = ActivityIndex::open(&dir.path().join("a.db")).unwrap();
        let stats = run(&config, &db).unwrap();
        assert_eq!(stats.files_scanned, 1);
        assert_eq!(stats.cards_inserted, 1, "fixture has exactly one failure");
        assert!(stats.failed.is_empty(), "fixture must parse cleanly");

        let cards = db.recent(&Default::default()).unwrap();
        assert_eq!(cards.len(), 1);
        let c = &cards[0];
        assert_eq!(c.help.as_ref().unwrap().template_id, "hook.plugin_missing");
    }

    /// The convergence invariant: after every full pass, the table
    /// reflects the on-disk JSONL state exactly. Each pass is a
    /// per-session delete-then-replay, so the counts shift between
    /// passes (first pass inserts; subsequent passes also insert
    /// because the slate is wiped first), but the row count
    /// converges.
    #[test]
    fn run_converges_to_jsonl_state_across_passes() {
        let dir = tempdir().unwrap();
        let config = dir.path().join("claude");
        let jsonl_path = config
            .join("projects")
            .join("-Users-x-proj")
            .join("sess.jsonl");
        let fixture = include_str!("testdata/hook_plugin_missing.jsonl").trim();
        write_jsonl(&jsonl_path, &[fixture]);

        let db = ActivityIndex::open(&dir.path().join("a.db")).unwrap();
        let first = run(&config, &db).unwrap();
        assert_eq!(first.cards_inserted, 1);
        assert_eq!(first.cards_pruned, 0);

        // Second pass: rebuild semantics — drops then re-inserts.
        // The user-visible invariant is the row count, not the
        // delta counts.
        let second = run(&config, &db).unwrap();
        assert_eq!(
            db.row_count().unwrap(),
            1,
            "table converges to one row per JSONL hook failure"
        );
        assert_eq!(second.cards_pruned, 0, "no stale sessions to prune");
    }

    /// Regression for Codex audit MEDIUM #2: a JSONL whose source
    /// file gets deleted between passes must have its cards swept
    /// from the index. Without the live-set sweep, deleted
    /// transcripts leave orphaned cards forever.
    #[test]
    fn run_prunes_cards_for_deleted_source_jsonl() {
        let dir = tempdir().unwrap();
        let config = dir.path().join("claude");
        let project = config.join("projects").join("-Users-x-proj");
        let path_a = project.join("a.jsonl");
        let path_b = project.join("b.jsonl");
        let fixture = include_str!("testdata/hook_plugin_missing.jsonl").trim();
        write_jsonl(&path_a, &[fixture]);
        write_jsonl(&path_b, &[fixture]);

        let db = ActivityIndex::open(&dir.path().join("a.db")).unwrap();
        let first = run(&config, &db).unwrap();
        assert_eq!(first.cards_inserted, 2);
        assert_eq!(db.row_count().unwrap(), 2);

        // Delete source `b`; rerun. The orphaned card must be swept.
        std::fs::remove_file(&path_b).unwrap();
        let second = run(&config, &db).unwrap();
        assert_eq!(second.cards_pruned, 1);
        assert_eq!(db.row_count().unwrap(), 1);
        let cards = db.recent(&Default::default()).unwrap();
        assert_eq!(cards[0].session_path, path_a);
    }

    /// Regression for Codex audit MEDIUM #2 (other half): a JSONL
    /// whose contents change between passes must reflect the new
    /// state — not stack new cards on top of old ones.
    #[test]
    fn run_replays_edited_jsonl_without_stacking() {
        let dir = tempdir().unwrap();
        let config = dir.path().join("claude");
        let jsonl_path = config
            .join("projects")
            .join("-Users-x-proj")
            .join("sess.jsonl");
        let fixture = include_str!("testdata/hook_plugin_missing.jsonl").trim();
        // First pass: two failures in the file.
        write_jsonl(&jsonl_path, &[fixture, fixture]);

        let db = ActivityIndex::open(&dir.path().join("a.db")).unwrap();
        let first = run(&config, &db).unwrap();
        // Both lines have the same uuid (same fixture), so insert
        // dedup leaves one row. The point is the table reflects
        // current state.
        assert_eq!(first.cards_inserted, 1);

        // Edit: rewrite the JSONL to be empty. The next pass must
        // wipe the row, not retain the prior state.
        std::fs::write(&jsonl_path, "").unwrap();
        let second = run(&config, &db).unwrap();
        assert_eq!(
            db.row_count().unwrap(),
            0,
            "edited-to-empty JSONL leaves zero rows"
        );
        // Empty content + still-present file = no prune (the file is
        // still in the live set), but the per-session delete-then-
        // replay yields zero new cards.
        assert_eq!(second.cards_pruned, 0);
    }

    #[test]
    fn missing_projects_dir_is_clean_zero_result() {
        let dir = tempdir().unwrap();
        let db = ActivityIndex::open(&dir.path().join("a.db")).unwrap();
        let stats = run(&dir.path().join("claude"), &db).unwrap();
        assert_eq!(stats.files_scanned, 0);
        assert_eq!(stats.cards_inserted, 0);
        assert!(stats.failed.is_empty());
    }

    /// Phase 4 plugin attribution survives the round-trip from
    /// classifier → bulk insert → recent() → row_to_card.
    /// Regression for the smoke-test bug where every queried card
    /// had plugin=None despite the classifier extracting it.
    #[test]
    fn plugin_attribution_round_trips_through_index() {
        let dir = tempdir().unwrap();
        let config = dir.path().join("claude");
        let jsonl = config
            .join("projects")
            .join("-Users-x-proj")
            .join("s.jsonl");
        let fixture = include_str!("testdata/hook_plugin_missing.jsonl").trim();
        write_jsonl(&jsonl, &[fixture]);

        let db = ActivityIndex::open(&dir.path().join("a.db")).unwrap();
        let stats = run(&config, &db).unwrap();
        assert_eq!(stats.cards_inserted, 1);

        let cards = db.recent(&Default::default()).unwrap();
        assert_eq!(cards.len(), 1);
        assert_eq!(
            cards[0].plugin.as_deref(),
            Some("mermaid-preview@xiaolai"),
            "plugin attribution must round-trip end-to-end"
        );

        // Filtering by plugin should also find it.
        let by_plugin = db
            .recent(&crate::activity::index::RecentQuery {
                plugin: Some("mermaid-preview".to_string()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(by_plugin.len(), 1, "plugin filter must match by bare name");
    }

    /// Regression for Codex audit verification PARTIAL: if the user
    /// deletes the whole `~/.claude/projects/` tree and reruns
    /// reindex, every existing index row is stale. The early-return
    /// path must sweep the table, not return cleanly with rows
    /// retained.
    #[test]
    fn missing_projects_dir_sweeps_all_existing_rows() {
        let dir = tempdir().unwrap();
        let config = dir.path().join("claude");
        let path_a = config
            .join("projects")
            .join("-Users-x-proj")
            .join("a.jsonl");
        let fixture = include_str!("testdata/hook_plugin_missing.jsonl").trim();
        write_jsonl(&path_a, &[fixture]);

        let db = ActivityIndex::open(&dir.path().join("a.db")).unwrap();
        let first = run(&config, &db).unwrap();
        assert_eq!(first.cards_inserted, 1);

        // Now wipe the whole projects/ tree.
        std::fs::remove_dir_all(config.join("projects")).unwrap();
        let second = run(&config, &db).unwrap();
        assert_eq!(second.cards_pruned, 1, "row swept on missing tree");
        assert_eq!(db.row_count().unwrap(), 0);
    }

    /// A single unparseable line must not crash backfill or skip
    /// adjacent valid lines on the same file.
    #[test]
    fn malformed_line_in_jsonl_is_skipped_silently() {
        let dir = tempdir().unwrap();
        let config = dir.path().join("claude");
        let jsonl_path = config
            .join("projects")
            .join("-Users-x-proj")
            .join("sess.jsonl");
        let fixture = include_str!("testdata/hook_plugin_missing.jsonl").trim();
        write_jsonl(&jsonl_path, &["{not json}", fixture, "garbage"]);

        let db = ActivityIndex::open(&dir.path().join("a.db")).unwrap();
        let stats = run(&config, &db).unwrap();
        assert_eq!(stats.files_scanned, 1);
        assert_eq!(stats.cards_inserted, 1, "valid line still classified");
        assert!(
            stats.failed.is_empty(),
            "malformed lines are not file-level failures"
        );
    }
}
