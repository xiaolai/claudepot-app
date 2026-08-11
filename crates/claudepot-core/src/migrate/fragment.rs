//! §5.6 / §5.7 — the two per-project fragments that carry CC state
//! living *outside* the slug tree.
//!
//! See `dev-docs/archive/project-migrate-spec.md` §3.2 (layout), §5.6
//! (`~/.claude.json`), §5.7 (`history.jsonl`), §6 (collision policy).
//!
//! Both are **additive** bundle entries: `SCHEMA_VERSION` is not
//! bumped for them. The writer records them in `file_inventory`, so a
//! newer bundle verifies on an older importer, which simply never
//! reads them; a newer importer treats a missing fragment as "old
//! bundle, nothing to merge". Bumping the schema instead would make
//! every existing bundle fail hard at the `UnsupportedSchemaVersion`
//! gate for a purely additive change.
//!
//! ## Locking
//!
//! The import holds the global import lock for P1..P8 (`mod.rs`),
//! mutually exclusive with rename / repair / move. Do **not** acquire
//! a second lock here — these functions are called from inside it.
//!
//! ## Why `project` is rewritten but not hashed
//!
//! A `history.jsonl` line carries an absolute `project` path, which
//! the substitution table rewrites on import. The dedupe key is
//! therefore `(sessionId, hash(prompt))` and never the whole line: an
//! entry that arrived before the rewrite and the same entry after it
//! are the same entry, and hashing the line would import a duplicate
//! of every one of them.

use crate::migrate::apply;
use crate::migrate::bundle::BundleWriter;
use crate::migrate::error::MigrateError;
use crate::migrate::plan::SubstitutionTable;
use crate::migrate::rewrite;
use serde_json::{Map, Value};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

pub const CLAUDE_JSON_FRAGMENT: &str = "claude-json-fragment.json";
pub const HISTORY_FRAGMENT: &str = "history-fragment.jsonl";

/// The `.claude.json` belonging to `config_dir`.
///
/// Deliberately derived from the passed config dir rather than via
/// `paths::resolved_global_claude_json()`: migrate is always given an
/// explicit config dir (a temp tree under test, a real one in
/// production), and a global resolver would read the developer's own
/// `~/.claude.json` during an export from a fixture.
///
/// Precedence mirrors CC's `getGlobalClaudeFile`: the legacy
/// `<config>/.config.json`, then `<config>/.claude.json` (the
/// `CLAUDE_CONFIG_DIR` layout), then the `~/.claude` sibling.
pub fn claude_json_for(config_dir: &Path) -> Option<PathBuf> {
    let legacy = config_dir.join(".config.json");
    if legacy.is_file() {
        return Some(legacy);
    }
    let inside = config_dir.join(".claude.json");
    if inside.is_file() {
        return Some(inside);
    }
    let sibling = config_dir.parent()?.join(".claude.json");
    sibling.is_file().then_some(sibling)
}

/// `history.jsonl` belonging to `config_dir`.
pub fn history_for(config_dir: &Path) -> PathBuf {
    config_dir.join("history.jsonl")
}

// ---------------------------------------------------------------------------
// Export side
// ---------------------------------------------------------------------------

/// Append both fragments for one project. Returns how many entries were
/// written (0..=2) so the caller can keep its file count honest.
///
/// Every absence is normal, not an error: a fresh machine has no
/// `~/.claude.json`, a project may have no entry in it, and a machine
/// that has never used the prompt history has no `history.jsonl`.
pub fn append_fragments(
    writer: &mut BundleWriter,
    project_id: &str,
    config_dir: &Path,
    source_cwd: &str,
    session_ids: &[String],
) -> Result<usize, MigrateError> {
    let mut written = 0;

    if let Some(bytes) = extract_claude_json_fragment(config_dir, source_cwd) {
        writer.append_bytes(
            &format!("projects/{project_id}/{CLAUDE_JSON_FRAGMENT}"),
            &bytes,
            0o600,
        )?;
        written += 1;
    }

    if let Some(bytes) = extract_history_fragment(config_dir, session_ids) {
        writer.append_bytes(
            &format!("projects/{project_id}/{HISTORY_FRAGMENT}"),
            &bytes,
            0o600,
        )?;
        written += 1;
    }

    Ok(written)
}

/// The source's `projects[<source_cwd>]` value, serialized. `None` when
/// the file is absent, unparseable, or carries no such key.
pub fn extract_claude_json_fragment(config_dir: &Path, source_cwd: &str) -> Option<Vec<u8>> {
    let path = claude_json_for(config_dir)?;
    let text = fs::read_to_string(&path).ok()?;
    let root: Value = serde_json::from_str(&text).ok()?;
    let value = root.get("projects")?.get(source_cwd)?;
    serde_json::to_vec_pretty(value).ok()
}

/// History lines whose `sessionId` is in `session_ids`, in source
/// order. `None` when the file is absent or nothing matches.
pub fn extract_history_fragment(config_dir: &Path, session_ids: &[String]) -> Option<Vec<u8>> {
    let text = fs::read_to_string(history_for(config_dir)).ok()?;
    let wanted: HashSet<&str> = session_ids.iter().map(|s| s.as_str()).collect();
    let mut out = String::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        // A line we cannot parse is not ours to claim; leave it home.
        let Ok(obj) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if obj
            .get("sessionId")
            .and_then(|v| v.as_str())
            .is_some_and(|s| wanted.contains(s))
        {
            out.push_str(line);
            out.push('\n');
        }
    }
    (!out.is_empty()).then(|| out.into_bytes())
}

// ---------------------------------------------------------------------------
// Import side
// ---------------------------------------------------------------------------

/// One applied fragment: the file touched, plus the snapshot of its
/// prior content for `undo`.
#[derive(Debug, Clone)]
pub struct FragmentStep {
    pub after: String,
    pub snapshot_path: Option<String>,
    /// Set for the `~/.claude.json` step: the `projects` key written.
    pub fragment_key: Option<String>,
}

/// §5.6 — re-key the fragment onto `projects[target_cwd]`, rewriting
/// embedded absolute paths through `table`.
///
/// Collision policy is §6's `merge`: shallow merge per top-level key,
/// imported wins, keys the import does not carry survive on the target.
pub fn apply_claude_json_fragment(
    staged_project_root: &Path,
    config_dir: &Path,
    target_cwd: &str,
    table: &SubstitutionTable,
    bundle_id: &str,
) -> Result<Option<FragmentStep>, MigrateError> {
    let staged = staged_project_root.join(CLAUDE_JSON_FRAGMENT);
    if !staged.is_file() {
        return Ok(None);
    }
    let bytes = fs::read(&staged).map_err(MigrateError::from)?;
    let mut fragment: Value =
        serde_json::from_slice(&bytes).map_err(|e| MigrateError::Serialize(e.to_string()))?;
    // Rewrites `activeWorktreeSession.originalCwd`, `.worktreePath`,
    // absolute-path `mcpServers[*].command`, and any other embedded
    // path — the table is the same one the slug tree used.
    rewrite::rewrite_value(&mut fragment, table);

    // Target file: the `~/.claude` sibling when it doesn't exist yet.
    let target = claude_json_for(config_dir).unwrap_or_else(|| {
        config_dir
            .parent()
            .unwrap_or(config_dir)
            .join(".claude.json")
    });
    let snapshot = apply::snapshot_file(bundle_id, &target)?;

    let mut root: Value = match fs::read_to_string(&target) {
        Ok(t) => serde_json::from_str(&t).unwrap_or_else(|_| Value::Object(Map::new())),
        Err(_) => Value::Object(Map::new()),
    };
    if !root.is_object() {
        root = Value::Object(Map::new());
    }
    let obj = root.as_object_mut().expect("object by construction");
    let projects = obj
        .entry("projects")
        .or_insert_with(|| Value::Object(Map::new()));
    if !projects.is_object() {
        *projects = Value::Object(Map::new());
    }
    let projects = projects.as_object_mut().expect("object by construction");

    match (projects.get(target_cwd), fragment) {
        // Shallow merge: imported wins per key, target keeps the rest.
        (Some(Value::Object(existing)), Value::Object(incoming)) => {
            let mut merged = existing.clone();
            for (k, v) in incoming {
                merged.insert(k, v);
            }
            projects.insert(target_cwd.to_string(), Value::Object(merged));
        }
        (_, incoming) => {
            projects.insert(target_cwd.to_string(), incoming);
        }
    }

    write_atomic(
        &target,
        &serde_json::to_vec_pretty(&root).map_err(|e| MigrateError::Serialize(e.to_string()))?,
    )?;

    Ok(Some(FragmentStep {
        after: target.to_string_lossy().to_string(),
        snapshot_path: snapshot.map(|p| p.to_string_lossy().to_string()),
        fragment_key: Some(target_cwd.to_string()),
    }))
}

/// §5.7 — append the history fragment, deduping by
/// `(sessionId, hash(prompt))`. Existing target lines are never
/// rewritten, reordered, or dropped; new lines land after them.
pub fn apply_history_fragment(
    staged_project_root: &Path,
    config_dir: &Path,
    table: &SubstitutionTable,
    bundle_id: &str,
) -> Result<Option<FragmentStep>, MigrateError> {
    let staged = staged_project_root.join(HISTORY_FRAGMENT);
    if !staged.is_file() {
        return Ok(None);
    }
    let incoming = fs::read_to_string(&staged).map_err(MigrateError::from)?;
    let target = history_for(config_dir);
    let existing = fs::read_to_string(&target).unwrap_or_default();

    let mut seen: HashSet<String> = existing.lines().filter_map(dedupe_key).collect();

    let mut appended = String::new();
    for line in incoming.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let (rewritten, _) = rewrite::rewrite_jsonl_line_multi(line, table);
        match dedupe_key(rewritten.as_str()) {
            // Unparseable or key-less lines are dropped rather than
            // appended blind — without a key we cannot tell a genuine
            // new entry from one already present, and re-importing
            // would grow the file without bound.
            None => continue,
            Some(k) => {
                if seen.insert(k) {
                    appended.push_str(&rewritten);
                    appended.push('\n');
                }
            }
        }
    }
    if appended.is_empty() {
        return Ok(None);
    }

    let snapshot = apply::snapshot_file(bundle_id, &target)?;
    let mut out = existing;
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&appended);
    write_atomic(&target, out.as_bytes())?;

    Ok(Some(FragmentStep {
        after: target.to_string_lossy().to_string(),
        snapshot_path: snapshot.map(|p| p.to_string_lossy().to_string()),
        fragment_key: None,
    }))
}

/// `(sessionId, hash(prompt))` per §5.7. `None` when the line is not
/// JSON or carries no `sessionId`.
fn dedupe_key(line: &str) -> Option<String> {
    let v: Value = serde_json::from_str(line).ok()?;
    let sid = v.get("sessionId")?.as_str()?;
    // CC names the prompt `display`. Absent, an empty prompt still
    // dedupes correctly against another empty prompt in the session.
    let prompt = v.get("display").and_then(|d| d.as_str()).unwrap_or("");
    use sha2::Digest;
    let mut h = sha2::Sha256::new();
    h.update(prompt.as_bytes());
    Some(format!("{sid}\u{0}{}", hex::encode(h.finalize())))
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), MigrateError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(MigrateError::from)?;
    }
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let mut tmp = tempfile::NamedTempFile::new_in(parent).map_err(MigrateError::from)?;
    std::io::Write::write_all(&mut tmp, bytes).map_err(MigrateError::from)?;
    tmp.persist(path).map_err(|e| MigrateError::from(e.error))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::migrate::plan::RuleOrigin;

    fn table(from: &str, to: &str) -> SubstitutionTable {
        let mut t = SubstitutionTable::new();
        t.push(from, to, RuleOrigin::ProjectCwd);
        t.finalize();
        t
    }

    fn cfg(tmp: &Path) -> PathBuf {
        let c = tmp.join(".claude");
        fs::create_dir_all(&c).unwrap();
        c
    }

    #[test]
    fn claude_json_prefers_config_dir_over_sibling() {
        let td = tempfile::tempdir().unwrap();
        let c = cfg(td.path());
        fs::write(td.path().join(".claude.json"), "{}").unwrap();
        assert_eq!(claude_json_for(&c).unwrap(), td.path().join(".claude.json"));
        fs::write(c.join(".claude.json"), "{}").unwrap();
        assert_eq!(claude_json_for(&c).unwrap(), c.join(".claude.json"));
    }

    #[test]
    fn extract_claude_json_returns_none_when_key_absent() {
        let td = tempfile::tempdir().unwrap();
        let c = cfg(td.path());
        fs::write(
            td.path().join(".claude.json"),
            r#"{"projects":{"/other":{"a":1}}}"#,
        )
        .unwrap();
        assert!(extract_claude_json_fragment(&c, "/mine").is_none());
        assert!(extract_claude_json_fragment(&c, "/other").is_some());
    }

    #[test]
    fn extract_history_filters_by_session_id() {
        let td = tempfile::tempdir().unwrap();
        let c = cfg(td.path());
        fs::write(
            history_for(&c),
            "{\"sessionId\":\"a\",\"display\":\"one\"}\n\
             {\"sessionId\":\"b\",\"display\":\"two\"}\n\
             not json\n",
        )
        .unwrap();
        let out = extract_history_fragment(&c, &["a".to_string()]).unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.contains("\"a\""));
        assert!(!s.contains("\"b\""));
        assert_eq!(s.lines().count(), 1);
    }

    #[test]
    fn extract_history_none_when_nothing_matches() {
        let td = tempfile::tempdir().unwrap();
        let c = cfg(td.path());
        fs::write(history_for(&c), "{\"sessionId\":\"z\"}\n").unwrap();
        assert!(extract_history_fragment(&c, &["a".to_string()]).is_none());
    }

    #[test]
    fn apply_claude_json_shallow_merges_imported_over_target() {
        let td = tempfile::tempdir().unwrap();
        let c = cfg(td.path());
        let staged = td.path().join("staged");
        fs::create_dir_all(&staged).unwrap();
        fs::write(
            staged.join(CLAUDE_JSON_FRAGMENT),
            r#"{"allowedTools":["imported"],"newKey":1}"#,
        )
        .unwrap();
        fs::write(
            td.path().join(".claude.json"),
            r#"{"projects":{"/tgt":{"allowedTools":["target"],"keepMe":true}},"other":9}"#,
        )
        .unwrap();

        let step = apply_claude_json_fragment(&staged, &c, "/tgt", &table("/src", "/tgt"), "b1")
            .unwrap()
            .unwrap();
        assert_eq!(step.fragment_key.as_deref(), Some("/tgt"));

        let root: Value =
            serde_json::from_str(&fs::read_to_string(td.path().join(".claude.json")).unwrap())
                .unwrap();
        let p = &root["projects"]["/tgt"];
        assert_eq!(p["allowedTools"][0], "imported", "imported wins per key");
        assert_eq!(p["keepMe"], true, "target-only key survives");
        assert_eq!(p["newKey"], 1);
        assert_eq!(root["other"], 9, "unrelated top-level keys untouched");
    }

    #[test]
    fn apply_claude_json_rewrites_embedded_paths() {
        let td = tempfile::tempdir().unwrap();
        let c = cfg(td.path());
        let staged = td.path().join("staged");
        fs::create_dir_all(&staged).unwrap();
        fs::write(
            staged.join(CLAUDE_JSON_FRAGMENT),
            r#"{"activeWorktreeSession":{"originalCwd":"/src/x"}}"#,
        )
        .unwrap();

        apply_claude_json_fragment(&staged, &c, "/tgt", &table("/src", "/tgt"), "b1").unwrap();

        let root: Value =
            serde_json::from_str(&fs::read_to_string(td.path().join(".claude.json")).unwrap())
                .unwrap();
        assert_eq!(
            root["projects"]["/tgt"]["activeWorktreeSession"]["originalCwd"],
            "/tgt/x"
        );
    }

    #[test]
    fn apply_history_appends_and_preserves_existing_order() {
        let td = tempfile::tempdir().unwrap();
        let c = cfg(td.path());
        let staged = td.path().join("staged");
        fs::create_dir_all(&staged).unwrap();
        fs::write(
            history_for(&c),
            "{\"sessionId\":\"old\",\"display\":\"first\"}\n",
        )
        .unwrap();
        fs::write(
            staged.join(HISTORY_FRAGMENT),
            "{\"sessionId\":\"new\",\"display\":\"second\",\"project\":\"/src\"}\n",
        )
        .unwrap();

        apply_history_fragment(&staged, &c, &table("/src", "/tgt"), "b1").unwrap();

        let out = fs::read_to_string(history_for(&c)).unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("\"old\""), "existing line stays first");
        assert!(lines[1].contains("\"new\""));
        assert!(lines[1].contains("/tgt"), "project path rewritten");
    }

    /// Re-importing the same bundle must add nothing — the property
    /// that makes repeated transport safe.
    #[test]
    fn apply_history_is_idempotent() {
        let td = tempfile::tempdir().unwrap();
        let c = cfg(td.path());
        let staged = td.path().join("staged");
        fs::create_dir_all(&staged).unwrap();
        fs::write(
            staged.join(HISTORY_FRAGMENT),
            "{\"sessionId\":\"s\",\"display\":\"hello\"}\n",
        )
        .unwrap();
        let t = table("/src", "/tgt");

        assert!(apply_history_fragment(&staged, &c, &t, "b1")
            .unwrap()
            .is_some());
        let once = fs::read_to_string(history_for(&c)).unwrap();
        assert!(apply_history_fragment(&staged, &c, &t, "b2")
            .unwrap()
            .is_none());
        assert_eq!(fs::read_to_string(history_for(&c)).unwrap(), once);
    }

    /// Same session, different prompt, is a different entry.
    #[test]
    fn apply_history_dedupes_on_prompt_not_session_alone() {
        let td = tempfile::tempdir().unwrap();
        let c = cfg(td.path());
        let staged = td.path().join("staged");
        fs::create_dir_all(&staged).unwrap();
        fs::write(history_for(&c), "{\"sessionId\":\"s\",\"display\":\"a\"}\n").unwrap();
        fs::write(
            staged.join(HISTORY_FRAGMENT),
            "{\"sessionId\":\"s\",\"display\":\"a\"}\n\
             {\"sessionId\":\"s\",\"display\":\"b\"}\n",
        )
        .unwrap();

        apply_history_fragment(&staged, &c, &table("/src", "/tgt"), "b1").unwrap();

        let out = fs::read_to_string(history_for(&c)).unwrap();
        assert_eq!(out.lines().count(), 2, "only the new prompt is appended");
    }

    #[test]
    fn missing_fragments_are_not_an_error() {
        let td = tempfile::tempdir().unwrap();
        let c = cfg(td.path());
        let staged = td.path().join("staged");
        fs::create_dir_all(&staged).unwrap();
        let t = table("/src", "/tgt");
        assert!(apply_claude_json_fragment(&staged, &c, "/tgt", &t, "b1")
            .unwrap()
            .is_none());
        assert!(apply_history_fragment(&staged, &c, &t, "b1")
            .unwrap()
            .is_none());
    }
}
