//! File-history backup repath — the §5.4 dual rewrite.
//!
//! See `dev-docs/project-migrate-spec.md` §5.4 and
//! `dev-docs/project-migrate-cc-research.md` §8.
//!
//! CC backs up edited files into
//! `~/.claude/file-history/<sessionId>/<sha256(filePath)[0..16]>@v<n>`.
//! The filename is content-addressed by the **source-machine absolute
//! path**. After migration the new absolute path on the target
//! produces a different sha256 — so we must:
//!
//!   1. Rename each on-disk backup to `<sha256(new_path)[0..16]>@v<n>`.
//!   2. Rewrite the `Record<absolute_path, FileHistoryBackup>` keys
//!      inside `FileHistorySnapshotMessage` JSONL records.
//!
//! Step 2 is handled by `rewrite::rewrite_value` (object-key rewrite).
//! This module owns step 1: scanning a `file-history/<sid>/` directory
//! for `<hex>@v<n>` files, recovering the original path → new path
//! mapping from the JSONL records, and renaming the files in place.

use crate::migrate::error::MigrateError;
use crate::migrate::plan::SubstitutionTable;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Compute CC's backup filename hash: `sha256(filePath)[0..16]`. Note
/// CC hashes UTF-8 bytes of the path string (Node.js `crypto.createHash`
/// + `update(string)` defaults to UTF-8); Rust's `String` is already
/// UTF-8, so a direct byte feed matches.
pub fn backup_hash(absolute_path: &str) -> String {
    let mut h = Sha256::new();
    h.update(absolute_path.as_bytes());
    let digest = h.finalize();
    hex::encode(&digest[..8]) // first 8 bytes = 16 hex chars
}

/// Parse a `<hex>@v<n>` filename. Returns `(hex, version)` or `None`
/// if the shape doesn't match.
pub fn parse_backup_name(name: &str) -> Option<(String, u32)> {
    let (hex, rest) = name.split_once('@')?;
    if hex.len() != 16 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let v_part = rest.strip_prefix('v')?;
    let n: u32 = v_part.parse().ok()?;
    Some((hex.to_string(), n))
}

/// Aggregated stats for a file-history repath pass.
#[derive(Debug, Default, Clone)]
pub struct FileHistoryStats {
    pub files_renamed: usize,
    pub files_skipped_unknown_path: usize,
    pub files_unchanged: usize,
}

/// Repath one `file-history/<sid>/` directory. The `path_index` maps
/// `source_hash → source_path`, recovered from the project's session
/// JSONLs by `build_path_index_from_jsonls`.
///
/// Behavior per file:
///   - Filename matches the `<hex>@v<n>` shape AND `<hex>` appears in
///     `path_index`: compute new path via `table.apply_path`, then
///     rename to `<sha256(new_path)[0..16]>@v<n>`. If the new hash
///     equals the old (path didn't change under any rule), the file
///     stays put.
///   - Filename matches the shape but the hash isn't in the index:
///     skip with a warning (counted in `files_skipped_unknown_path`).
///     This can happen if the bundle was built with a session not in
///     the file-history dir, or vice versa.
///   - Filename doesn't match: leave alone (might be a `.tmp` or
///     similar; not our problem).
pub fn repath_dir(
    dir: &Path,
    table: &SubstitutionTable,
    path_index: &HashMap<String, String>,
) -> Result<FileHistoryStats, MigrateError> {
    let mut stats = FileHistoryStats::default();
    if !dir.exists() {
        return Ok(stats);
    }
    for entry in fs::read_dir(dir).map_err(MigrateError::from)? {
        let entry = entry.map_err(MigrateError::from)?;
        if !entry.file_type().map_err(MigrateError::from)?.is_file() {
            continue;
        }
        let name_os = entry.file_name();
        let name = name_os.to_string_lossy().to_string();
        let Some((source_hash, version)) = parse_backup_name(&name) else {
            continue;
        };
        let Some(source_path) = path_index.get(&source_hash) else {
            stats.files_skipped_unknown_path += 1;
            tracing::warn!(
                file = %name,
                "file-history backup with no matching JSONL record; skipped"
            );
            continue;
        };
        let new_path = match table.apply_path(source_path) {
            Some(np) => np,
            None => {
                stats.files_unchanged += 1;
                continue;
            }
        };
        let new_hash = backup_hash(&new_path);
        if new_hash == source_hash {
            stats.files_unchanged += 1;
            continue;
        }
        let new_name = format!("{new_hash}@v{version}");
        let target = entry.path().with_file_name(&new_name);
        if target.exists() {
            // Imported file collision — remove the existing one to
            // honor §6 "imported wins on file-history overlap".
            fs::remove_file(&target).map_err(MigrateError::from)?;
        }
        fs::rename(entry.path(), &target).map_err(MigrateError::from)?;
        stats.files_renamed += 1;
    }
    Ok(stats)
}

/// Walk a project's session JSONLs and pull the
/// `Record<absolute_path, ...>` keys out of every
/// `FileHistorySnapshotMessage` (`type: "file-history-snapshot"`).
/// Returns `source_hash → source_path`.
///
/// The walker ignores files that aren't `*.jsonl` and lines that
/// don't parse — this is best-effort recovery, not validation.
pub fn build_path_index_from_jsonls(
    jsonl_paths: &[PathBuf],
) -> Result<HashMap<String, String>, MigrateError> {
    let mut index = HashMap::new();
    use std::io::{BufRead, BufReader};
    for path in jsonl_paths {
        let file = match fs::File::open(path) {
            Ok(f) => f,
            Err(e) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "migrate::file_history: skipping unreadable JSONL"
                );
                continue;
            }
        };
        let reader = BufReader::new(file);
        for (lineno, line) in reader.lines().enumerate() {
            let line = match line {
                Ok(l) => l,
                Err(e) => {
                    tracing::warn!(
                        path = %path.display(),
                        line = lineno + 1,
                        error = %e,
                        "migrate::file_history: I/O error reading line"
                    );
                    break;
                }
            };
            let v: Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(
                        path = %path.display(),
                        line = lineno + 1,
                        error = %e,
                        "migrate::file_history: skipping unparseable JSONL line",
                    );
                    continue;
                }
            };
            collect_paths(&v, &mut index);
        }
    }
    Ok(index)
}

fn collect_paths(v: &Value, out: &mut HashMap<String, String>) {
    if let Value::Object(map) = v {
        // Be tolerant of CC's nesting: `snapshot.trackedFileBackups` or
        // a top-level `trackedFileBackups`. Just look for any object
        // whose values are `{ ..., version: <n> }`-shaped.
        if let Some(Value::Object(backups)) = map.get("trackedFileBackups") {
            for key in backups.keys() {
                let h = backup_hash(key);
                out.entry(h).or_insert_with(|| key.clone());
            }
        }
        for (_, child) in map {
            collect_paths(child, out);
        }
    } else if let Value::Array(arr) = v {
        for child in arr {
            collect_paths(child, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrate::plan::RuleOrigin;

    #[test]
    fn parse_valid_backup_name() {
        assert_eq!(
            parse_backup_name("0123456789abcdef@v1"),
            Some(("0123456789abcdef".to_string(), 1))
        );
        assert_eq!(
            parse_backup_name("ffffffffffffffff@v42"),
            Some(("ffffffffffffffff".to_string(), 42))
        );
    }

    #[test]
    fn reject_short_hash() {
        assert!(parse_backup_name("0123@v1").is_none());
    }

    #[test]
    fn reject_non_hex_hash() {
        assert!(parse_backup_name("0123456789abcdeg@v1").is_none());
    }

    #[test]
    fn reject_missing_v_prefix() {
        assert!(parse_backup_name("0123456789abcdef@1").is_none());
    }

    #[test]
    fn backup_hash_deterministic() {
        let a = backup_hash("/Users/joker/x/foo.rs");
        let b = backup_hash("/Users/joker/x/foo.rs");
        assert_eq!(a, b);
        assert_eq!(a.len(), 16);
    }

    #[test]
    fn backup_hash_different_for_different_paths() {
        let a = backup_hash("/Users/joker/x/foo.rs");
        let b = backup_hash("/home/alice/x/foo.rs");
        assert_ne!(a, b);
    }

    #[test]
    fn repath_renames_file_when_path_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("sid");
        fs::create_dir_all(&dir).unwrap();

        let source_path = "/Users/joker/x/foo.rs";
        let source_hash = backup_hash(source_path);
        let backup_name = format!("{source_hash}@v1");
        fs::write(dir.join(&backup_name), "diff").unwrap();

        let mut table = SubstitutionTable::new();
        table.push("/Users/joker", "/home/alice", RuleOrigin::Home);
        table.finalize();

        let mut index = HashMap::new();
        index.insert(source_hash.clone(), source_path.to_string());

        let stats = repath_dir(&dir, &table, &index).unwrap();
        assert_eq!(stats.files_renamed, 1);

        let new_hash = backup_hash("/home/alice/x/foo.rs");
        assert!(dir.join(format!("{new_hash}@v1")).exists());
        assert!(!dir.join(&backup_name).exists());
    }

    #[test]
    fn repath_leaves_unchanged_when_no_rule_matches() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("sid");
        fs::create_dir_all(&dir).unwrap();
        let source_path = "/elsewhere/foo.rs";
        let source_hash = backup_hash(source_path);
        let backup_name = format!("{source_hash}@v3");
        fs::write(dir.join(&backup_name), "x").unwrap();

        let mut table = SubstitutionTable::new();
        table.push("/Users/joker", "/home/alice", RuleOrigin::Home);
        table.finalize();
        let mut index = HashMap::new();
        index.insert(source_hash.clone(), source_path.to_string());
        let stats = repath_dir(&dir, &table, &index).unwrap();
        assert_eq!(stats.files_unchanged, 1);
        assert_eq!(stats.files_renamed, 0);
        assert!(dir.join(&backup_name).exists());
    }

    #[test]
    fn repath_skips_files_with_unknown_hash() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("sid");
        fs::create_dir_all(&dir).unwrap();
        // Synthetic name with a hash we have no JSONL record for.
        fs::write(dir.join("0123456789abcdef@v7"), "x").unwrap();
        let table = SubstitutionTable::new();
        let stats = repath_dir(&dir, &table, &HashMap::new()).unwrap();
        assert_eq!(stats.files_skipped_unknown_path, 1);
    }

    #[test]
    fn build_path_index_from_jsonls_collects_keys() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("a.jsonl");
        fs::write(
            &p,
            r#"{"type":"file-history-snapshot","snapshot":{"trackedFileBackups":{"/x/foo.rs":{"v":1}}}}
{"type":"other"}
"#,
        )
        .unwrap();
        let idx = build_path_index_from_jsonls(&[p]).unwrap();
        let h = backup_hash("/x/foo.rs");
        assert_eq!(idx.get(&h), Some(&"/x/foo.rs".to_string()));
    }
}
