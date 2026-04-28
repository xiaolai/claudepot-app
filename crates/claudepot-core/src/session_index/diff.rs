//! Pure diff between "what's on disk" and "what's in the cache".
//!
//! Kept pure so it's cheap to test exhaustively and so the refresh
//! path in `mod.rs` stays readable. No I/O, no SQLite, no file
//! system — just the tuple-level comparison.
//!
//! Guard granularity is `(size, mtime_ns, inode)`. Content hashes
//! would be ideal but too expensive; the triple catches every
//! realistic edit: CC append-rewrites bump mtime + size, while
//! `session_move` atomic-swaps bump inode even when size + mtime
//! happen to match. On non-Unix platforms inode is always 0 and the
//! guard degrades to `(size, mtime_ns)`. The escape hatch for
//! pathological "all three collided" scenarios is
//! `SessionIndex::rebuild`.

use std::collections::HashMap;

/// One `(path, size, mtime, inode)` tuple — same shape for both the
/// fs side and the cached side of the comparison. `inode` is 0 on
/// platforms / filesystems where it's not available.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IndexTuple {
    pub file_path: String,
    pub size: u64,
    pub mtime_ns: i64,
    pub inode: u64,
}

/// What the refresh pass needs to do to converge the cache with disk.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct DiffPlan {
    /// Paths that need a full `scan_session`. Includes both brand-new
    /// files and files whose `(size, mtime_ns)` diverged from the
    /// cached value.
    pub to_upsert: Vec<String>,
    /// Paths present in the cache but no longer on disk.
    pub to_delete: Vec<String>,
}

/// Compare two tuple sets and produce an apply plan. Deterministic —
/// outputs are sorted so tests and SQL transactions stay stable.
pub fn diff_fs_vs_db(fs: &[IndexTuple], db: &[IndexTuple]) -> DiffPlan {
    let db_by_path: HashMap<&str, &IndexTuple> =
        db.iter().map(|t| (t.file_path.as_str(), t)).collect();
    let fs_paths: HashMap<&str, &IndexTuple> =
        fs.iter().map(|t| (t.file_path.as_str(), t)).collect();

    let mut to_upsert: Vec<String> = Vec::new();
    for t in fs {
        match db_by_path.get(t.file_path.as_str()) {
            None => to_upsert.push(t.file_path.clone()),
            Some(cached)
                if cached.size != t.size
                    || cached.mtime_ns != t.mtime_ns
                    || cached.inode != t.inode =>
            {
                to_upsert.push(t.file_path.clone());
            }
            _ => {}
        }
    }

    let mut to_delete: Vec<String> = db
        .iter()
        .filter(|t| !fs_paths.contains_key(t.file_path.as_str()))
        .map(|t| t.file_path.clone())
        .collect();

    to_upsert.sort();
    to_delete.sort();
    DiffPlan {
        to_upsert,
        to_delete,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(path: &str, size: u64, mtime_ns: i64) -> IndexTuple {
        ti(path, size, mtime_ns, 0)
    }

    fn ti(path: &str, size: u64, mtime_ns: i64, inode: u64) -> IndexTuple {
        IndexTuple {
            file_path: path.into(),
            size,
            mtime_ns,
            inode,
        }
    }

    #[test]
    fn empty_both_sides() {
        let plan = diff_fs_vs_db(&[], &[]);
        assert!(plan.to_upsert.is_empty());
        assert!(plan.to_delete.is_empty());
    }

    #[test]
    fn all_new_files_fall_into_upsert() {
        let fs = vec![t("/a.jsonl", 10, 1), t("/b.jsonl", 20, 2)];
        let plan = diff_fs_vs_db(&fs, &[]);
        assert_eq!(plan.to_upsert, vec!["/a.jsonl", "/b.jsonl"]);
        assert!(plan.to_delete.is_empty());
    }

    #[test]
    fn matching_tuples_noop() {
        let fs = vec![t("/a.jsonl", 10, 1)];
        let db = vec![t("/a.jsonl", 10, 1)];
        assert_eq!(diff_fs_vs_db(&fs, &db), DiffPlan::default());
    }

    #[test]
    fn mtime_bump_triggers_upsert() {
        let fs = vec![t("/a.jsonl", 10, 99)];
        let db = vec![t("/a.jsonl", 10, 1)];
        let plan = diff_fs_vs_db(&fs, &db);
        assert_eq!(plan.to_upsert, vec!["/a.jsonl"]);
        assert!(plan.to_delete.is_empty());
    }

    #[test]
    fn size_bump_triggers_upsert_even_if_mtime_unchanged() {
        // Can't happen normally but guards against a filesystem that
        // truncates mtime resolution coarser than we expect.
        let fs = vec![t("/a.jsonl", 99, 1)];
        let db = vec![t("/a.jsonl", 10, 1)];
        let plan = diff_fs_vs_db(&fs, &db);
        assert_eq!(plan.to_upsert, vec!["/a.jsonl"]);
    }

    #[test]
    fn missing_from_fs_falls_into_delete() {
        let fs: Vec<IndexTuple> = vec![];
        let db = vec![t("/a.jsonl", 10, 1), t("/b.jsonl", 20, 2)];
        let plan = diff_fs_vs_db(&fs, &db);
        assert!(plan.to_upsert.is_empty());
        assert_eq!(plan.to_delete, vec!["/a.jsonl", "/b.jsonl"]);
    }

    #[test]
    fn mixed_changes_partition_correctly() {
        let fs = vec![
            t("/kept.jsonl", 10, 1),    // unchanged
            t("/bumped.jsonl", 20, 99), // mtime changed
            t("/new.jsonl", 30, 3),     // new
        ];
        let db = vec![
            t("/kept.jsonl", 10, 1),
            t("/bumped.jsonl", 20, 1),
            t("/gone.jsonl", 40, 4),
        ];
        let plan = diff_fs_vs_db(&fs, &db);
        assert_eq!(plan.to_upsert, vec!["/bumped.jsonl", "/new.jsonl"]);
        assert_eq!(plan.to_delete, vec!["/gone.jsonl"]);
    }

    #[test]
    fn inode_bump_triggers_upsert_even_when_size_and_mtime_match() {
        // session_move rewrites a transcript via create-temp-and-rename.
        // Size and mtime can match by chance; inode tells the truth.
        let fs = vec![ti("/a.jsonl", 10, 1, 999)];
        let db = vec![ti("/a.jsonl", 10, 1, 42)];
        let plan = diff_fs_vs_db(&fs, &db);
        assert_eq!(plan.to_upsert, vec!["/a.jsonl"]);
    }

    #[test]
    fn outputs_are_sorted_for_determinism() {
        let fs = vec![
            t("/c.jsonl", 1, 1),
            t("/a.jsonl", 1, 1),
            t("/b.jsonl", 1, 1),
        ];
        let plan = diff_fs_vs_db(&fs, &[]);
        assert_eq!(
            plan.to_upsert,
            vec!["/a.jsonl", "/b.jsonl", "/c.jsonl"],
            "upsert output must be sorted"
        );
    }
}
