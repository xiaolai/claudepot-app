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

/// Write a snapshot of a pre-existing config value (P7). Filename format:
/// `<timestamp-unix-millis>-<new_san>-P7.json`.
fn write_snapshot(
    snapshots_dir: &Path,
    new_san: &str,
    value: &Value,
) -> Result<PathBuf, ProjectError> {
    let json = serde_json::to_string_pretty(value)
        .map_err(|e| ProjectError::Io(std::io::Error::other(e.to_string())))?;
    write_phase_snapshot(snapshots_dir, new_san, "P7", json.as_bytes())
}

/// Write a phase snapshot to `<dir>/<ts>-<safe_san>-<phase>.json`, 0600
/// on Unix. The single writer behind P7's config-value snapshots and
/// P10's raw-text registry snapshot — the two differ only in payload
/// (pretty-printed `Value` vs verbatim bytes) and phase suffix.
///
/// Snapshots can contain project-scoped secrets (trust flags, MCP
/// tokens, history), so permissions are restricted after creation.
fn write_phase_snapshot(
    snapshots_dir: &Path,
    new_san: &str,
    phase: &str,
    bytes: &[u8],
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

    let path = snapshots_dir.join(format!("{ts}-{safe_san}-{phase}.json"));
    fs::write(&path, bytes).map_err(ProjectError::Io)?;
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
// Phase 10: global plugin registry (plugins/installed_plugins.json)
// ---------------------------------------------------------------------------

/// Outcome of a Phase 10 rewrite.
#[derive(Debug, Default)]
pub struct PluginRegistryRewriteResult {
    /// Number of `projectPath` bindings rewritten from `old_path` (or a
    /// descendant) to `new_path`.
    pub bindings_rewritten: usize,
    /// Snapshot of the pre-rewrite registry file, written only when at
    /// least one binding changed. `None` on a no-op.
    pub snapshot_path: Option<PathBuf>,
}

/// Rewrite project-scoped plugin bindings in
/// `<config_dir>/plugins/installed_plugins.json` on a project move.
///
/// Claude Code records every installed plugin as
/// `plugins[<name>][] = { scope, projectPath, installPath, … }`. For a
/// `scope:"project"` (or `"local"`) install, `projectPath` is the
/// ABSOLUTE path of the project that installed it — the key Claude Code
/// uses to decide which plugins a project has. That binding lives in
/// this GLOBAL registry, not inside the project directory, so a project
/// move never carries it: the moved project's plugins silently appear
/// uninstalled because Claude Code finds no install record at the new
/// path. This phase repoints those bindings.
///
/// Semantics — schema-aware, only `projectPath` on `project`/`local`
/// records is touched (same boundary rule as P6/P7, via
/// `rewrite_path_string`):
///   - `projectPath == old_path` (the project root) → `new_path`.
///   - `projectPath` under `old_path` (a nested install root) →
///     rewritten prefix.
///   - `installPath` (global plugin cache) and `user`-scope bindings are
///     never touched — we mutate exactly the field a project move owns.
///
/// Before mutating, the original file is snapshotted to
/// `<snapshots_dir>/<ts>-<new_san>-P10.json` (the registry holds many
/// projects' bindings; a bad rewrite must be recoverable). No-op —
/// `Ok` with zero bindings and no snapshot — when the registry is
/// absent or holds no binding to `old_path`.
/// Fast-path gate: could `old_path` appear in this registry text at all?
/// Uses the JSON-escaped needle (Audit M11) so a Windows path (with `\\`
/// escaped in JSON) still matches, plus the raw form. A cheap `contains`
/// pre-check to skip the parse; the schema-aware walk is authoritative.
/// Shared by [`rewrite_installed_plugins`] and
/// [`installed_plugins_would_rewrite`] so their skip logic can't drift.
fn old_path_may_appear(contents: &str, old_path: &str) -> bool {
    let old_escaped = serde_json::to_string(old_path).unwrap_or_else(|_| format!("\"{old_path}\""));
    let needle = old_escaped.trim_matches('"');
    contents.contains(needle) || contents.contains(old_path)
}

pub fn rewrite_installed_plugins(
    registry_path: &Path,
    snapshots_dir: &Path,
    old_path: &str,
    new_path: &str,
    new_san: &str,
) -> Result<PluginRegistryRewriteResult, ProjectError> {
    let mut result = PluginRegistryRewriteResult::default();

    if !registry_path.exists() {
        tracing::debug!("installed_plugins.json absent; P10 no-op");
        return Ok(result);
    }

    let contents = fs::read_to_string(registry_path).map_err(ProjectError::Io)?;

    // Fast path: skip the parse when `old_path` can't appear.
    if !old_path_may_appear(&contents, old_path) {
        return Ok(result);
    }

    let mut root: Value = match serde_json::from_str(&contents) {
        Ok(v) => v,
        Err(e) => {
            return Err(ProjectError::Ambiguous(format!(
                "installed_plugins.json is not valid JSON: {e}"
            )));
        }
    };

    // Repoint schema-aware: walk `plugins.<name>[]`, gate on scope, and
    // rewrite ONLY `projectPath` via the boundary rule. A whole-document
    // string walk is also correct for today's schema (installPath lives
    // in the global cache, never under a project path), but P10's blast
    // radius is the ENTIRE registry — every project's bindings — so
    // touching only the field a move owns keeps a future path-bearing
    // field, or a `user`-scope record, from being caught. Same scope gate
    // the dry-run preview and the stale-binding detector use.
    let mut bindings = 0usize;
    if let Some(plugins) = root.get_mut("plugins").and_then(Value::as_object_mut) {
        for records in plugins.values_mut() {
            let Some(arr) = records.as_array_mut() else {
                continue;
            };
            for rec in arr.iter_mut() {
                // Immutable borrow to decide + compute the new value; it
                // ends before the mutable re-borrow below (NLL).
                let Some(pp) = project_binding_path(rec) else {
                    continue;
                };
                let Some(newp) =
                    crate::project_rewrite::rewrite_path_string(pp, old_path, new_path)
                else {
                    continue;
                };
                if let Some(Value::String(slot)) = rec.get_mut("projectPath") {
                    *slot = newp;
                    bindings += 1;
                }
            }
        }
    }
    if bindings == 0 {
        // Needle matched but no project/local binding resolved to
        // `old_path` under the boundary rule. Nothing to persist.
        return Ok(result);
    }

    // Snapshot the ORIGINAL bytes before the atomic replace.
    result.snapshot_path = Some(write_phase_snapshot(
        snapshots_dir,
        new_san,
        "P10",
        contents.as_bytes(),
    )?);
    write_config_atomic(registry_path, &root)?;
    result.bindings_rewritten = bindings;

    tracing::info!(
        bindings,
        "P10 installed_plugins.json plugin bindings rewritten"
    );
    Ok(result)
}

/// The `projectPath` of a `project`- or `local`-scoped plugin record, if
/// this record is a project binding a move owns. `None` for `user`-scope
/// (global, not tied to a project dir), malformed, or path-less records.
/// Single source of truth for "which bindings P10 considers" — shared by
/// the rewrite, the dry-run preview, and the stale-binding detector.
fn project_binding_path(rec: &Value) -> Option<&str> {
    let obj = rec.as_object()?;
    match obj.get("scope").and_then(Value::as_str) {
        Some("project") | Some("local") => {}
        _ => return None,
    }
    obj.get("projectPath")
        .and_then(Value::as_str)
        .filter(|p| !p.is_empty())
}

/// Whether the plugin registry holds at least one project/local binding
/// whose `projectPath` matches `old_path` (root or descendant) under the
/// boundary rule — exactly the set [`rewrite_installed_plugins`] would
/// repoint. The dry-run preview uses this instead of a raw substring
/// scan, which false-matches `/a/b` inside `/a/b-c`. Best-effort: any
/// read/parse failure returns `false` (the preview under-reports rather
/// than blocking a dry run).
pub(crate) fn installed_plugins_would_rewrite(registry_path: &Path, old_path: &str) -> bool {
    let Ok(contents) = fs::read_to_string(registry_path) else {
        return false;
    };
    // Fast path: skip the parse when `old_path` can't appear.
    if !old_path_may_appear(&contents, old_path) {
        return false;
    }
    let Ok(root) = serde_json::from_str::<Value>(&contents) else {
        return false;
    };
    let Some(plugins) = root.get("plugins").and_then(Value::as_object) else {
        return false;
    };
    plugins.values().any(|records| {
        records.as_array().is_some_and(|arr| {
            arr.iter().any(|rec| {
                // `rewrite_path_string(pp, old, old)` is Some iff `pp` is
                // `old` exactly or a descendant under the boundary rule —
                // a match predicate with no rewriting effect.
                project_binding_path(rec).is_some_and(|pp| {
                    crate::project_rewrite::rewrite_path_string(pp, old_path, old_path).is_some()
                })
            })
        })
    })
}

/// A project-scoped plugin binding whose `projectPath` no longer exists
/// on disk — the plugin registry still claims the plugin is installed for
/// a project directory that is gone (deleted or moved externally). This
/// is the plugin-side analogue of an orphaned transcript slug
/// (`session::move_::detect_orphaned_projects`): the same "a project
/// moved and CC's global, path-keyed state didn't follow" failure, on the
/// dimension that transcript-orphan detection does not cover.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StalePluginBinding {
    /// Registry key, e.g. `"sample@acme"`.
    pub plugin: String,
    /// The `projectPath` that no longer exists on disk.
    pub project_path: String,
    /// The install scope (`"project"` or `"local"`).
    pub scope: String,
}

/// Scan the plugin registry for project-scoped bindings whose
/// `projectPath` directory no longer exists on disk.
///
/// This is the detection half of the manual-move repair story. A project
/// moved with `claudepot project move` has its bindings repointed by P10;
/// a project moved **externally** (Finder / `mv` / `git`) leaves the
/// bindings pointing at a vanished path, and the moved project's plugins
/// silently stop resolving. This surfaces exactly those bindings so the
/// user can run `project move <old> <new>` (which repoints them) — the
/// plugin dimension that `detect_orphaned_projects` (transcripts only)
/// never reports.
///
/// Only `scope: "project"` and `"local"` bindings are considered — a
/// `"user"`-scoped install is global and not tied to any single project
/// directory's existence. Returns `[]` when the registry is absent, is
/// not the expected shape, or every project binding still resolves.
pub fn detect_stale_plugin_bindings(
    registry_path: &Path,
) -> Result<Vec<StalePluginBinding>, ProjectError> {
    if !registry_path.exists() {
        return Ok(Vec::new());
    }
    let contents = fs::read_to_string(registry_path).map_err(ProjectError::Io)?;
    let root: Value = match serde_json::from_str(&contents) {
        Ok(v) => v,
        // A malformed registry is a real problem, but detection is a
        // read-only health check — surface it rather than aborting a
        // caller that may be listing many things.
        Err(e) => {
            return Err(ProjectError::Ambiguous(format!(
                "installed_plugins.json is not valid JSON: {e}"
            )));
        }
    };

    let Some(plugins) = root.get("plugins").and_then(Value::as_object) else {
        return Ok(Vec::new());
    };

    let mut out = Vec::new();
    for (plugin, records) in plugins {
        let Some(arr) = records.as_array() else {
            continue;
        };
        for rec in arr {
            let Some(project_path) = project_binding_path(rec) else {
                continue;
            };
            // Stale iff the bound directory is CONFIRMED gone.
            // `try_exists()` distinguishes "absent" from "couldn't stat"
            // (permission/IO); on an error we do NOT flag — a binding we
            // can't verify is not reported as orphaned.
            if matches!(Path::new(project_path).try_exists(), Ok(false)) {
                let scope = rec
                    .get("scope")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                out.push(StalePluginBinding {
                    plugin: plugin.clone(),
                    project_path: project_path.to_string(),
                    scope,
                });
            }
        }
    }
    Ok(out)
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

        // Use platform-native absolute paths so `Path::is_absolute`
        // returns true on both OSes — Windows considers `/foo` to be
        // *relative* (no drive letter), so the Unix literal would
        // make the rewrite branch silently skip on the Windows runner.
        #[cfg(unix)]
        let (old, new, mem_old, mem_new) = (
            "/old/project",
            "/new/path",
            "/old/project/memdir",
            "/new/path/memdir",
        );
        #[cfg(windows)]
        let (old, new, mem_old, mem_new) = (
            r"C:\old\project",
            r"C:\new\path",
            r"C:\old\project\memdir",
            r"C:\new\path\memdir",
        );

        let initial = json!({"autoMemoryDirectory": mem_old, "other": "keep"});
        fs::write(&settings, initial.to_string()).unwrap();

        let rewrote = rewrite_project_settings(&proj, old, new).unwrap();
        assert!(rewrote);
        let after: Value = serde_json::from_str(&fs::read_to_string(&settings).unwrap()).unwrap();
        assert_eq!(after["autoMemoryDirectory"], json!(mem_new));
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

    // ── Phase 10: installed_plugins.json plugin-binding rewrite ──────────

    /// One install record shaped like Claude Code's real schema.
    fn plugin_record(scope: &str, project_path: &str, install_path: &str) -> Value {
        json!({
            "scope": scope,
            "projectPath": project_path,
            "installPath": install_path,
            "version": "0.5.1",
            "installedAt": "2026-07-14T00:36:29.392Z",
            "lastUpdated": "2026-07-14T00:36:29.392Z",
            "gitCommitSha": "2607c7afac185dfefe6148b214e9e186c2859ad0"
        })
    }

    fn snap_count(dir: &Path) -> usize {
        fs::read_dir(dir).map(|d| d.count()).unwrap_or(0)
    }

    #[test]
    fn installed_plugins_missing_file_is_noop() {
        let tmp = tempfile::tempdir().unwrap();
        let snap = tmp.path().join("snaps");
        let reg = tmp.path().join("installed_plugins.json");
        let r = rewrite_installed_plugins(&reg, &snap, "/a/b", "/c/d", "-c-d").unwrap();
        assert_eq!(r.bindings_rewritten, 0);
        assert!(r.snapshot_path.is_none());
    }

    #[test]
    fn installed_plugins_no_binding_to_old_path_is_noop() {
        let tmp = tempfile::tempdir().unwrap();
        let snap = tmp.path().join("snaps");
        let reg = tmp.path().join("installed_plugins.json");
        write_json(
            &reg,
            &json!({"version": 2, "plugins": {
                "foo@owner": [plugin_record("project", "/some/other/project", "/cache/foo")]
            }}),
        );
        let r = rewrite_installed_plugins(&reg, &snap, "/a/b", "/c/d", "-c-d").unwrap();
        assert_eq!(r.bindings_rewritten, 0);
        assert!(r.snapshot_path.is_none(), "no-op must not snapshot");
        // File untouched.
        assert_eq!(
            read_json(&reg)["plugins"]["foo@owner"][0]["projectPath"],
            json!("/some/other/project")
        );
    }

    #[test]
    fn exact_root_project_path_is_rewritten() {
        // The reported bug: projectPath equals the project ROOT exactly
        // (no trailing separator), which the boundary-prefix rule alone
        // would miss — rewrite_path_string's exact-match arm covers it.
        let tmp = tempfile::tempdir().unwrap();
        let snap = tmp.path().join("snaps");
        let reg = tmp.path().join("installed_plugins.json");
        write_json(
            &reg,
            &json!({"version": 2, "plugins": {
                "sample@acme": [plugin_record(
                    "project",
                    "/Users/dev/work/old-project",
                    "/Users/dev/.claude/plugins/cache/acme/sample/1.0.0",
                )]
            }}),
        );
        let r = rewrite_installed_plugins(
            &reg,
            &snap,
            "/Users/dev/work/old-project",
            "/Users/dev/work/renamed-project",
            "-Users-dev-work-renamed-project",
        )
        .unwrap();
        assert_eq!(r.bindings_rewritten, 1);
        let after = read_json(&reg);
        assert_eq!(
            after["plugins"]["sample@acme"][0]["projectPath"],
            json!("/Users/dev/work/renamed-project")
        );
        // installPath (global cache) is NOT under the project path → untouched.
        assert_eq!(
            after["plugins"]["sample@acme"][0]["installPath"],
            json!("/Users/dev/.claude/plugins/cache/acme/sample/1.0.0")
        );
    }

    #[test]
    fn descendant_project_path_is_rewritten() {
        let tmp = tempfile::tempdir().unwrap();
        let snap = tmp.path().join("snaps");
        let reg = tmp.path().join("installed_plugins.json");
        write_json(
            &reg,
            &json!({"version": 2, "plugins": {
                "foo@owner": [plugin_record("local", "/a/b/nested", "/cache/foo")]
            }}),
        );
        let r = rewrite_installed_plugins(&reg, &snap, "/a/b", "/c/d", "-c-d").unwrap();
        assert_eq!(r.bindings_rewritten, 1);
        assert_eq!(
            read_json(&reg)["plugins"]["foo@owner"][0]["projectPath"],
            json!("/c/d/nested")
        );
    }

    #[test]
    fn multiple_bindings_across_plugins_all_rewritten() {
        let tmp = tempfile::tempdir().unwrap();
        let snap = tmp.path().join("snaps");
        let reg = tmp.path().join("installed_plugins.json");
        write_json(
            &reg,
            &json!({"version": 2, "plugins": {
                "a@o": [plugin_record("project", "/a/b", "/cache/a")],
                "b@o": [plugin_record("project", "/a/b", "/cache/b")],
                // A user-scoped record for a DIFFERENT project — must not move.
                "c@o": [plugin_record("user", "/other", "/cache/c")],
            }}),
        );
        let r = rewrite_installed_plugins(&reg, &snap, "/a/b", "/c/d", "-c-d").unwrap();
        assert_eq!(r.bindings_rewritten, 2);
        let after = read_json(&reg);
        assert_eq!(after["plugins"]["a@o"][0]["projectPath"], json!("/c/d"));
        assert_eq!(after["plugins"]["b@o"][0]["projectPath"], json!("/c/d"));
        assert_eq!(after["plugins"]["c@o"][0]["projectPath"], json!("/other"));
    }

    #[test]
    fn rewrite_snapshots_the_original_then_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let snap = tmp.path().join("snaps");
        let reg = tmp.path().join("installed_plugins.json");
        let original = json!({"version": 2, "plugins": {
            "a@o": [plugin_record("project", "/a/b", "/cache/a")]
        }});
        write_json(&reg, &original);
        let original_text = fs::read_to_string(&reg).unwrap();

        let r1 = rewrite_installed_plugins(&reg, &snap, "/a/b", "/c/d", "-c-d").unwrap();
        assert_eq!(r1.bindings_rewritten, 1);
        let snap_path = r1.snapshot_path.expect("a rewrite must snapshot");
        assert!(snap_path.to_string_lossy().ends_with("-P10.json"));
        // Snapshot holds the verbatim pre-rewrite bytes.
        assert_eq!(fs::read_to_string(&snap_path).unwrap(), original_text);
        assert_eq!(snap_count(&snap), 1);

        // Second run: old_path no longer present → no-op, no new snapshot.
        let r2 = rewrite_installed_plugins(&reg, &snap, "/a/b", "/c/d", "-c-d").unwrap();
        assert_eq!(r2.bindings_rewritten, 0);
        assert!(r2.snapshot_path.is_none());
        assert_eq!(snap_count(&snap), 1, "idempotent re-run must not snapshot");
    }

    #[test]
    fn windows_drive_path_binding_is_rewritten() {
        // Golden pure-string test on a Windows shape (runs on every host
        // per rules/paths.md). Exact-root match, backslash separators.
        let tmp = tempfile::tempdir().unwrap();
        let snap = tmp.path().join("snaps");
        let reg = tmp.path().join("installed_plugins.json");
        write_json(
            &reg,
            &json!({"version": 2, "plugins": {
                "a@o": [plugin_record(
                    "project",
                    r"C:\Users\dev\old-project",
                    r"C:\Users\dev\.claude\plugins\cache\a",
                )]
            }}),
        );
        let r = rewrite_installed_plugins(
            &reg,
            &snap,
            r"C:\Users\dev\old-project",
            r"C:\Users\dev\renamed-project",
            "-c-d",
        )
        .unwrap();
        assert_eq!(r.bindings_rewritten, 1);
        let after = read_json(&reg);
        assert_eq!(
            after["plugins"]["a@o"][0]["projectPath"],
            json!(r"C:\Users\dev\renamed-project")
        );
        // Cache installPath untouched.
        assert_eq!(
            after["plugins"]["a@o"][0]["installPath"],
            json!(r"C:\Users\dev\.claude\plugins\cache\a")
        );
    }

    #[test]
    fn windows_unc_descendant_binding_is_rewritten() {
        let tmp = tempfile::tempdir().unwrap();
        let snap = tmp.path().join("snaps");
        let reg = tmp.path().join("installed_plugins.json");
        write_json(
            &reg,
            &json!({"version": 2, "plugins": {
                "a@o": [plugin_record("project", r"\\server\share\proj\sub", "/cache/a")]
            }}),
        );
        let r = rewrite_installed_plugins(
            &reg,
            &snap,
            r"\\server\share\proj",
            r"\\server\share\renamed",
            "-c-d",
        )
        .unwrap();
        assert_eq!(r.bindings_rewritten, 1);
        assert_eq!(
            read_json(&reg)["plugins"]["a@o"][0]["projectPath"],
            json!(r"\\server\share\renamed\sub")
        );
    }

    #[test]
    fn unparseable_registry_errors() {
        let tmp = tempfile::tempdir().unwrap();
        let snap = tmp.path().join("snaps");
        let reg = tmp.path().join("installed_plugins.json");
        // Contains the needle so the fast path doesn't skip it, but is
        // not valid JSON → hard error, not a silent no-op.
        fs::write(&reg, "{ not json /a/b").unwrap();
        let err = rewrite_installed_plugins(&reg, &snap, "/a/b", "/c/d", "-c-d").unwrap_err();
        assert!(matches!(err, ProjectError::Ambiguous(_)));
    }

    // ── detect_stale_plugin_bindings ─────────────────────────────────────

    #[test]
    fn detect_stale_missing_registry_is_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let reg = tmp.path().join("installed_plugins.json");
        assert!(detect_stale_plugin_bindings(&reg).unwrap().is_empty());
    }

    #[test]
    fn detect_stale_flags_gone_project_paths_only() {
        let tmp = tempfile::tempdir().unwrap();
        let reg = tmp.path().join("installed_plugins.json");
        // A live project dir that exists, and a gone one.
        let live = tmp.path().join("live-proj");
        fs::create_dir(&live).unwrap();
        let gone = tmp.path().join("gone-proj"); // never created

        write_json(
            &reg,
            &json!({"version": 2, "plugins": {
                "live@o":  [plugin_record("project", live.to_str().unwrap(), "/cache/live")],
                "gone@o":  [plugin_record("project", gone.to_str().unwrap(), "/cache/gone")],
                "local@o": [plugin_record("local",   gone.to_str().unwrap(), "/cache/local")],
                // user-scope binding to a gone path is global → NOT flagged.
                "user@o":  [plugin_record("user",    gone.to_str().unwrap(), "/cache/user")],
            }}),
        );

        let mut stale = detect_stale_plugin_bindings(&reg).unwrap();
        stale.sort_by(|a, b| a.plugin.cmp(&b.plugin));
        assert_eq!(
            stale.len(),
            2,
            "only project+local bindings to the gone path"
        );
        assert_eq!(stale[0].plugin, "gone@o");
        assert_eq!(stale[0].scope, "project");
        assert_eq!(stale[0].project_path, gone.to_string_lossy());
        assert_eq!(stale[1].plugin, "local@o");
        assert_eq!(stale[1].scope, "local");
        // The live and user-scope bindings are not reported.
        assert!(!stale.iter().any(|s| s.plugin == "live@o"));
        assert!(!stale.iter().any(|s| s.plugin == "user@o"));
    }

    #[test]
    fn detect_stale_all_live_is_empty() {
        let tmp = tempfile::tempdir().unwrap();
        let reg = tmp.path().join("installed_plugins.json");
        let live = tmp.path().join("p");
        fs::create_dir(&live).unwrap();
        write_json(
            &reg,
            &json!({"version": 2, "plugins": {
                "a@o": [plugin_record("project", live.to_str().unwrap(), "/cache/a")]
            }}),
        );
        assert!(detect_stale_plugin_bindings(&reg).unwrap().is_empty());
    }

    // ── schema-awareness (audit-fix round 1) ────────────────────────────

    #[test]
    fn rewrite_leaves_user_scope_binding_untouched() {
        // A user-scope binding is global; even if its projectPath equals
        // old_path, a project move must not move it.
        let tmp = tempfile::tempdir().unwrap();
        let snap = tmp.path().join("snaps");
        let reg = tmp.path().join("installed_plugins.json");
        write_json(
            &reg,
            &json!({"version": 2, "plugins": {
                "proj@o": [plugin_record("project", "/a/b", "/cache/p")],
                "user@o": [plugin_record("user",    "/a/b", "/cache/u")],
            }}),
        );
        let r = rewrite_installed_plugins(&reg, &snap, "/a/b", "/c/d", "-c-d").unwrap();
        assert_eq!(
            r.bindings_rewritten, 1,
            "only the project-scope binding moves"
        );
        let after = read_json(&reg);
        assert_eq!(after["plugins"]["proj@o"][0]["projectPath"], json!("/c/d"));
        assert_eq!(
            after["plugins"]["user@o"][0]["projectPath"],
            json!("/a/b"),
            "user-scope binding must stay put"
        );
    }

    #[test]
    fn rewrite_touches_only_project_path_field() {
        // A path-bearing field other than projectPath (a hypothetical
        // future field) under old_path must NOT be rewritten — a
        // whole-document string walk would have corrupted it.
        let tmp = tempfile::tempdir().unwrap();
        let snap = tmp.path().join("snaps");
        let reg = tmp.path().join("installed_plugins.json");
        write_json(
            &reg,
            &json!({"version": 2, "plugins": {
                "a@o": [{
                    "scope": "project",
                    "projectPath": "/a/b",
                    "installPath": "/cache/a",
                    "someFuturePath": "/a/b/sub/thing",
                    "version": "1.0.0"
                }]
            }}),
        );
        let r = rewrite_installed_plugins(&reg, &snap, "/a/b", "/c/d", "-c-d").unwrap();
        assert_eq!(r.bindings_rewritten, 1);
        let after = read_json(&reg);
        assert_eq!(after["plugins"]["a@o"][0]["projectPath"], json!("/c/d"));
        assert_eq!(
            after["plugins"]["a@o"][0]["someFuturePath"],
            json!("/a/b/sub/thing"),
            "non-projectPath fields are never rewritten"
        );
    }

    #[test]
    fn would_rewrite_is_schema_aware_not_substring() {
        let tmp = tempfile::tempdir().unwrap();
        let reg = tmp.path().join("installed_plugins.json");
        // A sibling that contains old as a substring but is not a
        // descendant, plus a user-scope exact match — neither counts.
        write_json(
            &reg,
            &json!({"version": 2, "plugins": {
                "sibling@o": [plugin_record("project", "/a/b-c", "/cache/s")],
                "user@o":    [plugin_record("user",    "/a/b",   "/cache/u")],
            }}),
        );
        assert!(
            !installed_plugins_would_rewrite(&reg, "/a/b"),
            "substring sibling + user-scope must not count as a rewrite"
        );

        // A genuine project binding at the exact old path does count.
        write_json(
            &reg,
            &json!({"version": 2, "plugins": {
                "real@o": [plugin_record("project", "/a/b", "/cache/r")],
            }}),
        );
        assert!(installed_plugins_would_rewrite(&reg, "/a/b"));
        // Missing registry → false, not a panic.
        assert!(!installed_plugins_would_rewrite(
            &tmp.path().join("nope.json"),
            "/a/b"
        ));
    }
}
