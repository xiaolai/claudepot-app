//! Multi-rule JSONL + JSON rewrite engine.
//!
//! See `dev-docs/project-migrate-spec.md` §5.5 and §5.4.
//!
//! Extends the single-rule prefix-match rewriter in
//! `project_rewrite::rewrite_path_string` to a full
//! `SubstitutionTable`. The rewrite walks JSON values and applies the
//! table to every `cwd`, `slug`, and string value (including object
//! keys for `FileHistorySnapshotMessage.trackedFileBackups`).
//!
//! Per-line semantics match `session_move_jsonl`:
//!   - Byte-exact off-target lines.
//!   - Mid-session `cd` preserved (each line's cwd is independently
//!     rewritten).
//!
//! Performance: `serde_json::Map` here uses `BTreeMap` by default,
//! which would reorder keys on round-trip. We accept the reorder
//! because (a) CC tolerates either ordering at parse time, and
//! (b) the alternative — surgical splice — multiplies code paths
//! across the multi-rule, multi-field surface. The reorder is a
//! one-time event at the migration boundary; resume-time CC writers
//! re-emit the file in their preferred order on the next turn.

use crate::migrate::error::MigrateError;
use crate::migrate::plan::SubstitutionTable;
use serde_json::Value;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;

/// Result of a single-file rewrite pass.
#[derive(Debug, Default, Clone)]
pub struct MultiRewriteStats {
    pub lines_rewritten: usize,
    pub fields_rewritten: usize,
}

/// Rewrite a JSONL file in place using the substitution table. Atomic
/// via tempfile + rename. Returns the number of lines that changed.
///
/// `target_slug` is recomputed per-line when a `cwd` is rewritten —
/// some message types carry both fields, and CC keeps them in sync.
pub fn rewrite_jsonl_multi(
    path: &Path,
    table: &SubstitutionTable,
) -> Result<MultiRewriteStats, MigrateError> {
    let src = fs::File::open(path).map_err(MigrateError::from)?;
    let reader = BufReader::new(src);
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let mut tmp = tempfile::NamedTempFile::new_in(parent).map_err(MigrateError::from)?;

    let mut stats = MultiRewriteStats::default();
    let mut any_change = false;

    for line in reader.lines() {
        let line = line.map_err(MigrateError::from)?;
        let (new_line, fields_changed) = rewrite_jsonl_line_multi(&line, table);
        if fields_changed > 0 {
            stats.lines_rewritten += 1;
            stats.fields_rewritten += fields_changed;
            any_change = true;
        }
        writeln!(tmp, "{new_line}").map_err(MigrateError::from)?;
    }

    if any_change {
        tmp.persist(path).map_err(|e| MigrateError::from(e.error))?;
    } else {
        drop(tmp);
    }
    Ok(stats)
}

/// Rewrite a single JSONL line. Cheap fast path: if no rule's `from`
/// occurs anywhere in the line, return as-is. Slow path: parse JSON,
/// walk, serialize.
///
/// Parse failures are logged at `warn` and the line is passed through
/// untouched. Silent skipping was the prior behavior; the audit
/// flagged it as opaque on corrupted JSONL — surfacing through tracing
/// makes the gap visible without aborting the whole rewrite (a single
/// torn line shouldn't sink an otherwise-clean migration).
pub fn rewrite_jsonl_line_multi(line: &str, table: &SubstitutionTable) -> (String, usize) {
    if !line_contains_any(line, table) {
        return (line.to_string(), 0);
    }
    let mut value: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(
                error = %e,
                len = line.len(),
                "migrate::rewrite: passing through unparseable JSONL line",
            );
            return (line.to_string(), 0);
        }
    };
    let count = rewrite_value(&mut value, table);
    if count == 0 {
        return (line.to_string(), 0);
    }
    match serde_json::to_string(&value) {
        Ok(s) => (s, count),
        Err(e) => {
            tracing::warn!(error = %e, "migrate::rewrite: serialize after rewrite failed");
            (line.to_string(), 0)
        }
    }
}

/// Apply the table to every string value AND every string key inside
/// a JSON value (recursively). Object keys must be rewritten because
/// `FileHistorySnapshotMessage.trackedFileBackups` is a
/// `Record<absolute_path, FileHistoryBackup>` keyed on source-machine
/// paths (§5.4).
///
/// A second pass also rewrites the `slug` field (when the same object
/// carries `cwd`) using `target_slug` of the rewritten cwd, to keep
/// CC's `SerializedMessage.slug` in sync with `cwd` across migration.
pub fn rewrite_value(v: &mut Value, table: &SubstitutionTable) -> usize {
    let mut count = 0;
    rewrite_value_inner(v, table, &mut count);
    count
}

fn rewrite_value_inner(v: &mut Value, table: &SubstitutionTable, count: &mut usize) {
    match v {
        Value::String(s) => {
            if let Some(next) = table.apply_path(s) {
                *s = next;
                *count += 1;
            }
        }
        Value::Object(map) => {
            // Two-pass: first rewrite keys (collect rewrites then apply),
            // then recurse into values.
            let key_rewrites: Vec<(String, String)> = map
                .keys()
                .filter_map(|k| table.apply_path(k).map(|new| (k.clone(), new)))
                .collect();

            // Audit fix: detect collisions before mutating. Two
            // distinct source keys can rewrite to the same target
            // (e.g. `/Users/joker/x` and `/Users/alice/x` both
            // mapped to `/home/joker/x` after worktree
            // consolidation). The pre-fix code applied them in
            // sequence and the second `map.insert` silently
            // overwrote the first — losing the value at the first
            // source key. We now leave both source keys in place
            // and log a warn; data is preserved, the un-rewritten
            // key surfaces as a broken absolute path on the target,
            // and the warning is the operator's signal to resolve
            // the conflict by hand.
            //
            // Two collision shapes are detected:
            //   1. Two `key_rewrites` entries share a `new_key`.
            //   2. A `new_key` already exists as a static (non-
            //      rewriting) key in `map`.
            use std::collections::{BTreeMap, BTreeSet};
            let mut new_key_targets: BTreeMap<&str, usize> = BTreeMap::new();
            for (_, new) in &key_rewrites {
                *new_key_targets.entry(new.as_str()).or_insert(0) += 1;
            }
            let old_key_set: BTreeSet<&str> =
                key_rewrites.iter().map(|(o, _)| o.as_str()).collect();

            for (old_key, new_key) in &key_rewrites {
                if old_key == new_key {
                    continue;
                }
                let dup_in_rewrites =
                    new_key_targets.get(new_key.as_str()).copied().unwrap_or(0) > 1;
                let collides_with_static =
                    map.contains_key(new_key) && !old_key_set.contains(new_key.as_str());
                if dup_in_rewrites || collides_with_static {
                    tracing::warn!(
                        old = %old_key,
                        new = %new_key,
                        "migrate::rewrite: object key collision — preserving original key, no rewrite applied"
                    );
                    continue;
                }
                if let Some(val) = map.remove(old_key) {
                    map.insert(new_key.clone(), val);
                    *count += 1;
                }
            }
            // Then recurse — values may themselves be strings or nested
            // objects/arrays.
            for (k, child) in map.iter_mut() {
                if k == "slug" {
                    // `slug` is already a sanitized form. We can't
                    // rewrite it via path-substitution; the apply
                    // phase recomputes it from the rewritten `cwd`
                    // sibling at the per-message level. Skip the
                    // recursive walk here so we don't accidentally
                    // splice path text into the slug.
                    continue;
                }
                rewrite_value_inner(child, table, count);
            }
            // Slug recomputation pass: if this object has both `cwd`
            // and `slug` strings, recompute `slug` from the rewritten
            // `cwd`. This is mandatory for CC's resume gate — `slug`
            // is used by `findProjectDir` lookup.
            if let (Some(Value::String(cwd_val)), Some(Value::String(slug_val))) =
                (map.get("cwd").cloned(), map.get_mut("slug"))
            {
                let recomputed = crate::migrate::plan::target_slug(&cwd_val);
                if *slug_val != recomputed {
                    *slug_val = recomputed;
                    *count += 1;
                }
            }
        }
        Value::Array(arr) => {
            for child in arr.iter_mut() {
                rewrite_value_inner(child, table, count);
            }
        }
        _ => {}
    }
}

/// Cheap pre-scan: does the line contain any rule's `from`? If not,
/// skip the JSON parse entirely. Uses the JSON-escaped form to handle
/// Windows backslashes that get doubled in the wire format.
fn line_contains_any(line: &str, table: &SubstitutionTable) -> bool {
    for rule in &table.rules {
        // Raw form (Unix paths typically appear verbatim in JSON
        // strings since they have no JSON-special chars).
        if line.contains(&rule.from) {
            return true;
        }
        // JSON-encoded form (handles backslash doubling on Windows).
        let encoded =
            serde_json::to_string(&rule.from).unwrap_or_else(|_| format!("\"{}\"", rule.from));
        let encoded_inner = encoded.trim_matches('"');
        if line.contains(encoded_inner) {
            return true;
        }
    }
    false
}

/// Rewrite a JSON file (e.g. `claude-json-fragment.json`) in memory
/// using the substitution table; write back atomically. Returns
/// number of fields rewritten.
pub fn rewrite_json_file(path: &Path, table: &SubstitutionTable) -> Result<usize, MigrateError> {
    let bytes = fs::read(path).map_err(MigrateError::from)?;
    let mut value: Value =
        serde_json::from_slice(&bytes).map_err(|e| MigrateError::Serialize(e.to_string()))?;
    let count = rewrite_value(&mut value, table);
    if count == 0 {
        return Ok(0);
    }
    let new_json =
        serde_json::to_vec_pretty(&value).map_err(|e| MigrateError::Serialize(e.to_string()))?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let mut tmp = tempfile::NamedTempFile::new_in(parent).map_err(MigrateError::from)?;
    tmp.write_all(&new_json).map_err(MigrateError::from)?;
    tmp.write_all(b"\n").map_err(MigrateError::from)?;
    tmp.persist(path).map_err(|e| MigrateError::from(e.error))?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrate::plan::RuleOrigin;

    fn unix_to_linux_table() -> SubstitutionTable {
        let mut t = SubstitutionTable::new();
        t.push("/Users/joker", "/home/alice", RuleOrigin::Home);
        t.push("/Users/joker/x", "/home/alice/x", RuleOrigin::ProjectCwd);
        t.finalize();
        t
    }

    #[test]
    fn rewrite_simple_cwd_in_object() {
        let table = unix_to_linux_table();
        let line = r#"{"cwd":"/Users/joker/x","msg":"hi"}"#;
        let (out, n) = rewrite_jsonl_line_multi(line, &table);
        assert!(n >= 1);
        assert!(out.contains(r#""cwd":"/home/alice/x""#));
    }

    #[test]
    fn rewrite_cwd_with_slug_recomputes_slug() {
        let table = unix_to_linux_table();
        // `slug` was the source slug; after migrate it must reflect
        // the target cwd.
        let line = r#"{"cwd":"/Users/joker/x","slug":"-Users-joker-x"}"#;
        let (out, n) = rewrite_jsonl_line_multi(line, &table);
        assert!(n >= 2);
        assert!(out.contains(r#""cwd":"/home/alice/x""#));
        assert!(out.contains(r#""slug":"-home-alice-x""#));
    }

    #[test]
    fn rewrite_off_target_line_unchanged() {
        let table = unix_to_linux_table();
        let line = r#"{"cwd":"/elsewhere"}"#;
        let (out, n) = rewrite_jsonl_line_multi(line, &table);
        assert_eq!(n, 0);
        assert_eq!(out, line);
    }

    #[test]
    fn rewrite_unparseable_line_unchanged() {
        let table = unix_to_linux_table();
        let line = "garbage not json /Users/joker";
        let (out, _n) = rewrite_jsonl_line_multi(line, &table);
        assert_eq!(out, line);
    }

    #[test]
    fn rewrite_object_keys_for_file_history() {
        // `FileHistorySnapshotMessage.snapshot.trackedFileBackups`
        // is `Record<absolute_path, FileHistoryBackup>` — keys are
        // source paths. Rewriter must rewrite the keys too.
        let table = unix_to_linux_table();
        let line = r#"{"trackedFileBackups":{"/Users/joker/x/foo.rs":{"v":1}}}"#;
        let (out, n) = rewrite_jsonl_line_multi(line, &table);
        assert!(n >= 1);
        assert!(out.contains(r#""/home/alice/x/foo.rs""#));
    }

    #[test]
    fn rewrite_jsonl_file_atomic_no_change_no_persist() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("a.jsonl");
        fs::write(&f, "{\"cwd\":\"/elsewhere\"}\n").unwrap();
        let mtime_before = fs::metadata(&f).unwrap().modified().unwrap();

        let table = unix_to_linux_table();
        let stats = rewrite_jsonl_multi(&f, &table).unwrap();
        assert_eq!(stats.lines_rewritten, 0);
        let mtime_after = fs::metadata(&f).unwrap().modified().unwrap();
        assert_eq!(mtime_before, mtime_after);
    }

    #[test]
    fn rewrite_jsonl_file_persists_on_change() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("a.jsonl");
        fs::write(
            &f,
            "{\"cwd\":\"/Users/joker/x\"}\n{\"cwd\":\"/elsewhere\"}\n",
        )
        .unwrap();

        let table = unix_to_linux_table();
        let stats = rewrite_jsonl_multi(&f, &table).unwrap();
        assert_eq!(stats.lines_rewritten, 1);

        let after = fs::read_to_string(&f).unwrap();
        assert!(after.contains(r#""cwd":"/home/alice/x""#));
        assert!(after.contains(r#""cwd":"/elsewhere""#));
    }

    #[test]
    fn rewrite_json_file_round_trips() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("frag.json");
        fs::write(
            &f,
            r#"{"originalCwd":"/Users/joker/x","worktreePath":"/Users/joker/x/wt"}"#,
        )
        .unwrap();
        let table = unix_to_linux_table();
        let n = rewrite_json_file(&f, &table).unwrap();
        assert!(n >= 2);
        let after = fs::read_to_string(&f).unwrap();
        assert!(after.contains("/home/alice/x"));
    }

    #[test]
    fn cross_os_unix_to_windows_rewrites_with_backslashes() {
        let mut t = SubstitutionTable::new();
        t.push(
            "/Users/joker/x",
            r"C:\Users\alice\x",
            RuleOrigin::ProjectCwd,
        );
        t.finalize();
        // Edit tool result on Unix: forward-slash boundary.
        let line = r#"{"cwd":"/Users/joker/x/src/main.rs"}"#;
        let (out, n) = rewrite_jsonl_line_multi(line, &t);
        assert!(n >= 1);
        // Cross-OS rewrite: the suffix separator is coerced to the
        // target's native form. Mixed separators (`C:\x/y`) are wrong
        // because CC writers don't produce them and downstream
        // sanitize/canonicalize would reject the path.
        assert!(
            out.contains(r#""cwd":"C:\\Users\\alice\\x\\src\\main.rs""#),
            "expected backslash-separated suffix; got: {out}"
        );
    }

    #[test]
    fn cross_os_windows_to_unix_rewrites_with_forward_slashes() {
        let mut t = SubstitutionTable::new();
        t.push(r"C:\Users\joker\x", "/home/alice/x", RuleOrigin::ProjectCwd);
        t.finalize();
        // Source Windows JSONL: backslash-doubled in JSON escaping.
        // The line as it appears in the file:
        let line = r#"{"cwd":"C:\\Users\\joker\\x\\src\\main.rs"}"#;
        let (out, n) = rewrite_jsonl_line_multi(line, &t);
        assert!(n >= 1, "expected rewrite; got: {out}");
        assert!(
            out.contains(r#""cwd":"/home/alice/x"#),
            "expected new cwd in output; got: {out}"
        );
    }

    #[test]
    fn longer_prefix_wins_in_jsonl_line() {
        // Both `/Users/joker` and `/Users/joker/x` apply; the longer
        // rule must win.
        let mut t = SubstitutionTable::new();
        t.push("/Users/joker", "/home/alice", RuleOrigin::Home);
        t.push("/Users/joker/x", "/srv/x", RuleOrigin::ProjectCwd);
        t.finalize();
        let line = r#"{"cwd":"/Users/joker/x/sub"}"#;
        let (out, _n) = rewrite_jsonl_line_multi(line, &t);
        // Project cwd rule wins because it's longer.
        assert!(out.contains(r#""cwd":"/srv/x/sub""#));
    }
}
