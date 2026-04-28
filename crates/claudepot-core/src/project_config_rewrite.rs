//! `~/.claude.json` project-map key rename for project rename (Phase 7).
//!
//! CC stores per-project settings (trust flags, allowedTools, MCP servers,
//! history, …) under `config.projects[<absolute-path>]`. When a project
//! is renamed, this map key must be migrated or the settings appear to
//! vanish from CC's perspective.
//!
//! Collision policy on pre-existing `projects[new_path]`:
//!   - Error (default): hard error, abort rename.
//!   - Merge (`--merge`): shallow merge, **old-wins** on top-level key
//!     collision. Inherits keys from new that old doesn't have.
//!   - Overwrite (`--overwrite`): drop target, use old value verbatim.
//!
//! Before any destructive change, the pre-existing value is snapshotted
//! to `<snapshots_dir>/<ts>-<new_san>-P7.json` so the user can recover
//! via `project repair --gc`-style retention (spec §6).
//!
//! After the key rename, nested absolute-path strings inside the moved
//! value are also prefix-rewritten (e.g. cached `cwd`) using the same
//! boundary rule as P6.

use crate::error::ProjectError;
use serde_json::Value;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// How to handle a pre-existing `projects[new_path]` entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigCollisionPolicy {
    /// Collision is an error; caller must re-run with Merge or Overwrite.
    Error,
    /// Shallow merge — `old_path`'s value wins on top-level key collision.
    /// Keys present only at `new_path` are inherited.
    Merge,
    /// Replace `new_path`'s value with `old_path`'s value verbatim.
    Overwrite,
}

/// Outcome of a Phase 7 rewrite.
#[derive(Debug, Default)]
pub struct ConfigRewriteResult {
    /// Whether `projects[old_path]` was found and moved to `projects[new_path]`.
    pub key_renamed: bool,
    /// Whether `projects[new_path]` already existed before the rename.
    pub had_collision: bool,
    /// For a merge: list of top-level keys where old and new both had
    /// a value and old won. Empty for non-collision or Overwrite cases.
    pub merged_keys: Vec<String>,
    /// For merge/overwrite collisions: path to the snapshot of the
    /// pre-existing `projects[new_path]` value. None if no collision.
    pub snapshot_path: Option<PathBuf>,
    /// Number of nested absolute-path strings rewritten inside the moved
    /// value (e.g. cached cwd entries within ProjectConfig).
    pub nested_rewrites: usize,
}

/// Key-rename `projects[old_path]` → `projects[new_path]` in
/// `~/.claude.json`. Snapshots the pre-existing target value before
/// destructive changes. Atomic replace of the config file.
pub fn rewrite_claude_json(
    config_path: &Path,
    snapshots_dir: &Path,
    old_path: &str,
    new_path: &str,
    new_san: &str,
    policy: ConfigCollisionPolicy,
) -> Result<ConfigRewriteResult, ProjectError> {
    let mut result = ConfigRewriteResult::default();

    if !config_path.exists() {
        tracing::debug!("~/.claude.json absent; P7 no-op");
        return Ok(result);
    }

    let contents = fs::read_to_string(config_path).map_err(ProjectError::Io)?;
    let mut root: Value = match serde_json::from_str(&contents) {
        Ok(v) => v,
        Err(e) => {
            return Err(ProjectError::Ambiguous(format!(
                "~/.claude.json is not valid JSON: {e}"
            )));
        }
    };

    let projects_map = match root.get_mut("projects") {
        Some(Value::Object(m)) => m,
        Some(_) => {
            tracing::debug!("config.projects is not an object; P7 no-op");
            return Ok(result);
        }
        None => {
            tracing::debug!("config.projects absent; P7 no-op");
            return Ok(result);
        }
    };

    let old_value = match projects_map.remove(old_path) {
        Some(v) => v,
        None => {
            tracing::debug!("projects[old_path] absent; P7 no-op");
            return Ok(result);
        }
    };

    // Rewrite nested paths inside the moved value BEFORE we re-insert.
    let mut moved_value = old_value;
    result.nested_rewrites =
        crate::project_rewrite::rewrite_strings_in_value_pub(&mut moved_value, old_path, new_path);

    // Collision handling on the target key.
    let new_value_to_insert = if let Some(existing) = projects_map.remove(new_path) {
        result.had_collision = true;
        // Snapshot the existing value before we touch it, so the user can
        // recover. Snapshot written even for Merge (not just Overwrite)
        // because Merge still destroys data where old-wins on collision.
        result.snapshot_path = Some(write_snapshot(snapshots_dir, new_san, &existing)?);

        match policy {
            ConfigCollisionPolicy::Error => {
                // Put state back where we found it.
                projects_map.insert(old_path.to_string(), moved_value);
                projects_map.insert(new_path.to_string(), existing);
                return Err(ProjectError::Ambiguous(format!(
                    "projects['{new_path}'] already exists in ~/.claude.json; \
                     re-run with --merge or --overwrite. \
                     Pre-existing value snapshotted to {:?}",
                    result.snapshot_path.as_ref()
                )));
            }
            ConfigCollisionPolicy::Merge => {
                // Shallow merge: old wins on top-level collision, new
                // contributes any keys old doesn't have.

                shallow_merge_old_wins(moved_value, existing, &mut result.merged_keys)
            }
            ConfigCollisionPolicy::Overwrite => moved_value,
        }
    } else {
        moved_value
    };

    projects_map.insert(new_path.to_string(), new_value_to_insert);
    result.key_renamed = true;

    // Atomic write-back preserving permissions.
    write_config_atomic(config_path, &root)?;

    tracing::info!(
        key_renamed = result.key_renamed,
        collision = result.had_collision,
        merged_keys = result.merged_keys.len(),
        nested_rewrites = result.nested_rewrites,
        "P7 ~/.claude.json rewrite complete"
    );
    Ok(result)
}

/// Shallow merge: for each top-level key, `old_val` wins. Keys that
/// exist only in `new_val` are inherited. Records overlapping keys in
/// `merged_keys` so the caller can print a loud collision notice.
///
/// If either value is not an object, returns `old_val` unchanged (we
/// don't know how to deep-merge non-object types meaningfully; the
/// snapshot preserves the dropped data).
fn shallow_merge_old_wins(old_val: Value, new_val: Value, merged_keys: &mut Vec<String>) -> Value {
    let (mut old_map, new_map) = match (old_val, new_val) {
        (Value::Object(o), Value::Object(n)) => (o, n),
        (old, _) => return old, // non-object: can't merge meaningfully; preserve old
    };

    for (k, v) in new_map {
        if old_map.contains_key(&k) {
            merged_keys.push(k);
            // old wins → drop new's value.
        } else {
            old_map.insert(k, v);
        }
    }
    Value::Object(old_map)
}

/// Write a snapshot of a pre-existing config value. Filename format:
/// `<timestamp-unix-millis>-<new_san>-P7.json`.
fn write_snapshot(
    snapshots_dir: &Path,
    new_san: &str,
    value: &Value,
) -> Result<PathBuf, ProjectError> {
    fs::create_dir_all(snapshots_dir).map_err(ProjectError::Io)?;

    // Sanitize new_san for use in the filename. Project sanitized names
    // are already [a-zA-Z0-9-], so this is belt-and-suspenders.
    let safe_san: String = new_san
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();

    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);

    let path = snapshots_dir.join(format!("{ts}-{safe_san}-P7.json"));
    let json = serde_json::to_string_pretty(value)
        .map_err(|e| ProjectError::Io(std::io::Error::other(e.to_string())))?;
    fs::write(&path, json).map_err(ProjectError::Io)?;
    // Snapshots can contain project-scoped secrets (trust flags, MCP
    // tokens, history). Restrict permissions on creation.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
    }
    Ok(path)
}

// ---------------------------------------------------------------------------
// Phase 9: project-local .claude/settings.json
// ---------------------------------------------------------------------------

/// Rewrite `autoMemoryDirectory` inside `<new_path>/.claude/settings.json`
/// if it is an absolute path that matches the old project path prefix.
///
/// Paths using `~/` or relative paths are already path-portable across
/// renames and need no rewrite. Only absolute paths anchored under
/// `old_path` are migrated.
///
/// Returns `true` if the file was rewritten, `false` otherwise (missing,
/// no `autoMemoryDirectory`, or the value doesn't match the old path).
pub fn rewrite_project_settings(
    new_project_path: &Path,
    old_path: &str,
    new_path: &str,
) -> Result<bool, ProjectError> {
    let settings_path = new_project_path.join(".claude").join("settings.json");
    if !settings_path.exists() {
        return Ok(false);
    }

    let contents = fs::read_to_string(&settings_path).map_err(ProjectError::Io)?;
    let mut value: Value = match serde_json::from_str(&contents) {
        Ok(v) => v,
        Err(_) => return Ok(false), // malformed; don't touch
    };

    let Some(obj) = value.as_object_mut() else {
        return Ok(false);
    };
    let Some(Value::String(current)) = obj.get_mut("autoMemoryDirectory") else {
        return Ok(false);
    };

    // Only rewrite if absolute and matches old_path prefix. Leave ~/…,
    // ./…, and absolute paths pointing elsewhere alone.
    let starts_with_tilde = current.starts_with('~');
    let is_absolute = Path::new(current.as_str()).is_absolute();
    if starts_with_tilde || !is_absolute {
        return Ok(false);
    }
    let Some(new_value) = crate::project_rewrite::rewrite_path_string(current, old_path, new_path)
    else {
        return Ok(false);
    };
    *current = new_value;

    write_config_atomic(&settings_path, &value)?;
    tracing::info!(
        file = ?settings_path,
        "P9 project-local settings.json autoMemoryDirectory rewritten"
    );
    Ok(true)
}

/// Atomic write preserving original file's Unix permissions if present.
fn write_config_atomic(path: &Path, value: &Value) -> Result<(), ProjectError> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let json = serde_json::to_string_pretty(value)
        .map_err(|e| ProjectError::Io(std::io::Error::other(e.to_string())))?;
    let mut tmp = tempfile::NamedTempFile::new_in(parent).map_err(ProjectError::Io)?;
    tmp.write_all(json.as_bytes()).map_err(ProjectError::Io)?;
    tmp.write_all(b"\n").map_err(ProjectError::Io)?;

    // Match the original file's permissions if it exists.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = fs::metadata(path) {
            let mode = meta.permissions().mode();
            let _ = fs::set_permissions(tmp.path(), fs::Permissions::from_mode(mode));
        }
    }

    tmp.persist(path).map_err(|e| ProjectError::Io(e.error))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn write_json(path: &Path, v: &Value) {
        fs::write(path, serde_json::to_string_pretty(v).unwrap()).unwrap();
    }

    fn read_json(path: &Path) -> Value {
        serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn test_rewrite_claude_json_missing_file_is_noop() {
        let tmp = tempfile::tempdir().unwrap();
        let snap = tmp.path().join("snaps");
        let cfg = tmp.path().join("claude.json");

        let r = rewrite_claude_json(
            &cfg,
            &snap,
            "/a/b",
            "/c/d",
            "-c-d",
            ConfigCollisionPolicy::Error,
        )
        .unwrap();
        assert!(!r.key_renamed);
        assert!(!r.had_collision);
    }

    #[test]
    fn test_rewrite_claude_json_no_projects_key_is_noop() {
        let tmp = tempfile::tempdir().unwrap();
        let snap = tmp.path().join("snaps");
        let cfg = tmp.path().join("claude.json");
        write_json(&cfg, &json!({"other": "stuff"}));

        let r = rewrite_claude_json(
            &cfg,
            &snap,
            "/a/b",
            "/c/d",
            "-c-d",
            ConfigCollisionPolicy::Error,
        )
        .unwrap();
        assert!(!r.key_renamed);
    }

    #[test]
    fn test_rewrite_claude_json_old_path_absent_is_noop() {
        let tmp = tempfile::tempdir().unwrap();
        let snap = tmp.path().join("snaps");
        let cfg = tmp.path().join("claude.json");
        write_json(&cfg, &json!({"projects": {"/elsewhere": {"trust": true}}}));

        let r = rewrite_claude_json(
            &cfg,
            &snap,
            "/a/b",
            "/c/d",
            "-c-d",
            ConfigCollisionPolicy::Error,
        )
        .unwrap();
        assert!(!r.key_renamed);
        // File should be untouched (content-wise at least).
        let after = read_json(&cfg);
        assert_eq!(after["projects"]["/elsewhere"]["trust"], json!(true));
    }

    #[test]
    fn test_rewrite_claude_json_clean_rename() {
        let tmp = tempfile::tempdir().unwrap();
        let snap = tmp.path().join("snaps");
        let cfg = tmp.path().join("claude.json");
        write_json(
            &cfg,
            &json!({
                "projects": {
                    "/a/b": {"allowedTools": ["Bash(git:*)"], "trust": true}
                },
                "otherTop": 42
            }),
        );

        let r = rewrite_claude_json(
            &cfg,
            &snap,
            "/a/b",
            "/c/d",
            "-c-d",
            ConfigCollisionPolicy::Error,
        )
        .unwrap();
        assert!(r.key_renamed);
        assert!(!r.had_collision);
        assert_eq!(r.merged_keys.len(), 0);
        assert!(r.snapshot_path.is_none());

        let after = read_json(&cfg);
        assert!(after["projects"].get("/a/b").is_none());
        assert_eq!(
            after["projects"]["/c/d"]["allowedTools"],
            json!(["Bash(git:*)"])
        );
        assert_eq!(after["projects"]["/c/d"]["trust"], json!(true));
        assert_eq!(after["otherTop"], json!(42));
    }

    #[test]
    fn test_rewrite_claude_json_collision_error_policy() {
        let tmp = tempfile::tempdir().unwrap();
        let snap = tmp.path().join("snaps");
        let cfg = tmp.path().join("claude.json");
        let before = json!({
            "projects": {
                "/a/b": {"trust": true},
                "/c/d": {"trust": false, "ghost": "yes"}
            }
        });
        write_json(&cfg, &before);

        let err = rewrite_claude_json(
            &cfg,
            &snap,
            "/a/b",
            "/c/d",
            "-c-d",
            ConfigCollisionPolicy::Error,
        )
        .unwrap_err();
        assert!(matches!(err, ProjectError::Ambiguous(_)));

        // State should be restored: both keys still present with
        // original values.
        let after = read_json(&cfg);
        assert_eq!(after["projects"]["/a/b"]["trust"], json!(true));
        assert_eq!(after["projects"]["/c/d"]["trust"], json!(false));
        assert_eq!(after["projects"]["/c/d"]["ghost"], json!("yes"));

        // Snapshot should have been written anyway so user can inspect.
        let snaps: Vec<_> = fs::read_dir(&snap)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert_eq!(snaps.len(), 1);
    }

    #[test]
    fn test_rewrite_claude_json_merge_old_wins() {
        let tmp = tempfile::tempdir().unwrap();
        let snap = tmp.path().join("snaps");
        let cfg = tmp.path().join("claude.json");
        write_json(
            &cfg,
            &json!({
                "projects": {
                    "/a/b": {"trust": true, "allowedTools": ["X"]},
                    "/c/d": {"trust": false, "ghostKey": "stay"}
                }
            }),
        );

        let r = rewrite_claude_json(
            &cfg,
            &snap,
            "/a/b",
            "/c/d",
            "-c-d",
            ConfigCollisionPolicy::Merge,
        )
        .unwrap();
        assert!(r.key_renamed);
        assert!(r.had_collision);
        assert_eq!(r.merged_keys, vec!["trust".to_string()]);
        assert!(r.snapshot_path.is_some());

        let after = read_json(&cfg);
        assert!(after["projects"].get("/a/b").is_none());
        // old wins on `trust`
        assert_eq!(after["projects"]["/c/d"]["trust"], json!(true));
        // old brings its own key
        assert_eq!(after["projects"]["/c/d"]["allowedTools"], json!(["X"]));
        // new's unique key is inherited
        assert_eq!(after["projects"]["/c/d"]["ghostKey"], json!("stay"));

        // Snapshot captures pre-existing new_path value.
        let snap_value: Value =
            serde_json::from_str(&fs::read_to_string(r.snapshot_path.unwrap()).unwrap()).unwrap();
        assert_eq!(snap_value["trust"], json!(false));
        assert_eq!(snap_value["ghostKey"], json!("stay"));
    }

    #[test]
    fn test_rewrite_claude_json_overwrite_policy() {
        let tmp = tempfile::tempdir().unwrap();
        let snap = tmp.path().join("snaps");
        let cfg = tmp.path().join("claude.json");
        write_json(
            &cfg,
            &json!({
                "projects": {
                    "/a/b": {"trust": true, "mark": "old"},
                    "/c/d": {"trust": false, "ghost": "discarded"}
                }
            }),
        );

        let r = rewrite_claude_json(
            &cfg,
            &snap,
            "/a/b",
            "/c/d",
            "-c-d",
            ConfigCollisionPolicy::Overwrite,
        )
        .unwrap();
        assert!(r.key_renamed);
        assert!(r.had_collision);
        assert!(r.snapshot_path.is_some());

        let after = read_json(&cfg);
        // Old's value replaces new entirely.
        assert_eq!(after["projects"]["/c/d"]["trust"], json!(true));
        assert_eq!(after["projects"]["/c/d"]["mark"], json!("old"));
        assert!(after["projects"]["/c/d"].get("ghost").is_none());
    }

    #[test]
    fn test_rewrite_claude_json_nested_path_rewrite() {
        let tmp = tempfile::tempdir().unwrap();
        let snap = tmp.path().join("snaps");
        let cfg = tmp.path().join("claude.json");
        let sep = std::path::MAIN_SEPARATOR;
        write_json(
            &cfg,
            &json!({
                "projects": {
                    "/a/b": {
                        "lastCwd": "/a/b",
                        "subCwd": format!("/a/b{sep}src"),
                        "unrelated": "/elsewhere",
                        "nested": {"deep": "/a/b"}
                    }
                }
            }),
        );

        let r = rewrite_claude_json(
            &cfg,
            &snap,
            "/a/b",
            "/c/d",
            "-c-d",
            ConfigCollisionPolicy::Error,
        )
        .unwrap();
        assert!(r.key_renamed);
        assert_eq!(r.nested_rewrites, 3); // lastCwd, subCwd, nested.deep

        let after = read_json(&cfg);
        let moved = &after["projects"]["/c/d"];
        assert_eq!(moved["lastCwd"], json!("/c/d"));
        assert_eq!(moved["subCwd"], json!(format!("/c/d{sep}src")));
        assert_eq!(moved["unrelated"], json!("/elsewhere"));
        assert_eq!(moved["nested"]["deep"], json!("/c/d"));
    }

    // P9 — project-local settings.json

    #[test]
    fn test_p9_rewrites_absolute_automem_path() {
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("new");
        fs::create_dir_all(proj.join(".claude")).unwrap();
        let settings = proj.join(".claude").join("settings.json");
        fs::write(
            &settings,
            r#"{"autoMemoryDirectory":"/old/project/memdir","other":"keep"}"#,
        )
        .unwrap();

        let rewrote = rewrite_project_settings(&proj, "/old/project", "/new/path").unwrap();
        assert!(rewrote);
        let after: Value = serde_json::from_str(&fs::read_to_string(&settings).unwrap()).unwrap();
        assert_eq!(after["autoMemoryDirectory"], json!("/new/path/memdir"));
        assert_eq!(after["other"], json!("keep"));
    }

    #[test]
    fn test_p9_skips_tilde_path() {
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("new");
        fs::create_dir_all(proj.join(".claude")).unwrap();
        let settings = proj.join(".claude").join("settings.json");
        fs::write(&settings, r#"{"autoMemoryDirectory":"~/memdir"}"#).unwrap();

        let rewrote = rewrite_project_settings(&proj, "/old", "/new").unwrap();
        assert!(!rewrote);
        // Unchanged.
        let after: Value = serde_json::from_str(&fs::read_to_string(&settings).unwrap()).unwrap();
        assert_eq!(after["autoMemoryDirectory"], json!("~/memdir"));
    }

    #[test]
    fn test_p9_skips_unrelated_absolute_path() {
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("new");
        fs::create_dir_all(proj.join(".claude")).unwrap();
        fs::write(
            proj.join(".claude").join("settings.json"),
            r#"{"autoMemoryDirectory":"/entirely/elsewhere"}"#,
        )
        .unwrap();

        let rewrote = rewrite_project_settings(&proj, "/old", "/new").unwrap();
        assert!(!rewrote);
    }

    #[test]
    fn test_p9_no_settings_file_is_noop() {
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("new");
        fs::create_dir(&proj).unwrap();

        let rewrote = rewrite_project_settings(&proj, "/old", "/new").unwrap();
        assert!(!rewrote);
    }

    #[test]
    fn test_p9_no_automem_key_is_noop() {
        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("new");
        fs::create_dir_all(proj.join(".claude")).unwrap();
        fs::write(
            proj.join(".claude").join("settings.json"),
            r#"{"theme":"dark"}"#,
        )
        .unwrap();

        let rewrote = rewrite_project_settings(&proj, "/old", "/new").unwrap();
        assert!(!rewrote);
    }

    #[test]
    fn test_rewrite_claude_json_invalid_json_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let snap = tmp.path().join("snaps");
        let cfg = tmp.path().join("claude.json");
        fs::write(&cfg, "{ not valid json").unwrap();

        let err = rewrite_claude_json(
            &cfg,
            &snap,
            "/a/b",
            "/c/d",
            "-c-d",
            ConfigCollisionPolicy::Error,
        )
        .unwrap_err();
        assert!(matches!(err, ProjectError::Ambiguous(_)));
    }
}
