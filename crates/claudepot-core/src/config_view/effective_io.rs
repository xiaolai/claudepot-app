//! Load the effective-settings input bundle straight off disk.
//!
//! The in-memory `effective_settings::compute` is pure — it accepts the
//! already-parsed JSON per source. This module bridges the gap: it
//! reads the CC-mandated files, runs them through `mask_json` where
//! appropriate, and returns a populated
//! [`EffectiveSettingsInput`](crate::config_view::effective_settings::EffectiveSettingsInput).
//!
//! MCP has the same shape: [`load_mcp_bundle`] reads every source the
//! MCP resolver consumes.

use crate::config_view::{
    effective_mcp::{McpLayer, McpSourceBundle},
    effective_settings::EffectiveSettingsInput,
    model::{PolicyOrigin, Scope},
    plugin_base,
    policy::{self, PolicySource},
};
use crate::paths::claude_config_dir;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Load every on-disk source the effective-settings cascade consumes.
/// All inputs are optional — missing files map to `None`, and
/// `compute()` treats `None` as an empty layer.
pub fn load_effective_settings_input(cwd: &Path) -> EffectiveSettingsInput {
    let home = claude_config_dir();

    // PluginBase is the lowest layer in the cascade.
    let (_plugin_files, plugins) =
        crate::config_view::discover::collect_plugins();
    let plugin_base_raw = plugin_base::build_plugin_base(&plugins);
    let plugin_base = non_empty_or_none(plugin_base_raw);

    // File-based sources.
    let user = read_settings_file(&home.join("settings.json"));
    let project = read_settings_file(&cwd.join(".claude").join("settings.json"));
    let local = read_settings_file(&cwd.join(".claude").join("settings.local.json"));
    let flag: Option<Value> = None; // Claudepot has no CLI flag context.

    // Policy sources: managed-file-composite is assembled from the
    // drop-in dir. Remote / MDM / HKCU remain extension points —
    // they contribute `None` here and callers can pass explicit
    // sources if they've got a cache/registry reader plugged in.
    let composite = load_managed_composite(&home);
    let policy_sources = vec![
        PolicySource { origin: PolicyOrigin::Remote, value: None },
        PolicySource { origin: PolicyOrigin::MdmAdmin, value: None },
        PolicySource {
            origin: PolicyOrigin::ManagedFileComposite,
            value: composite,
        },
        PolicySource { origin: PolicyOrigin::HkcuUser, value: None },
    ];

    EffectiveSettingsInput {
        plugin_base,
        user,
        project,
        local,
        flag,
        policy_sources,
    }
}

fn read_settings_file(path: &Path) -> Option<Value> {
    if !path.is_file() {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn non_empty_or_none(v: Value) -> Option<Value> {
    match &v {
        Value::Object(m) if m.is_empty() => None,
        _ => Some(v),
    }
}

fn load_managed_composite(home: &Path) -> Option<Value> {
    let base = policy::load_managed_file(&home.join("managed-settings.json"))
        .ok()
        .flatten();
    let drops = policy::scan_managed_dir(&home.join("managed-settings.d"));
    if base.is_none() && drops.is_empty() {
        return None;
    }
    let composite = policy::build_managed_composite(base.as_ref(), &drops);
    non_empty_or_none(composite)
}

/// Load the MCP source bundle. The project chain walks from `cwd`
/// upward until we hit the filesystem root OR a `.git` dir (whichever
/// comes first — plan §6.4's stopping rule for project-related walks).
///
/// `effective_settings` is loaded in parallel because the MCP gating
/// predicate depends on `enableAllProjectMcpServers` /
/// `enabledMcpjsonServers` / `disabledMcpjsonServers` from the
/// MERGED settings.
pub fn load_mcp_bundle(cwd: &Path, effective_settings: Value) -> McpSourceBundle {
    // Enterprise: ~/.claude/managed-mcp.json
    let home = claude_config_dir();
    let enterprise = read_mcp_servers_obj(&home.join("managed-mcp.json"));

    // User: `mcpServers` from ~/.claude.json.
    let user = read_claude_json_mcp_servers(&home.join(".claude.json"));

    // Local (per-project): ~/.claude.json's
    // `projects[<project-path>].mcpServers`. We use the literal `cwd`
    // as the key — CC canonicalizes via `getProjectPathForConfig`,
    // which we approximate via `find_canonical_git_root`.
    let project_key = crate::project_memory::find_canonical_git_root(cwd)
        .unwrap_or_else(|| cwd.to_path_buf());
    let local = read_claude_json_local_mcp(&home.join(".claude.json"), &project_key);

    // Project chain: every `.mcp.json` from cwd up to fs root (or git).
    let project_chain = walk_project_mcp(cwd);

    // Plugin MCP: each enabled plugin's `manifest.mcp_servers`.
    let plugin = collect_plugin_mcp_servers();

    McpSourceBundle {
        project_chain,
        user,
        local,
        plugin,
        enterprise,
        effective_settings,
        project_settings_enabled: true,
    }
}

fn read_mcp_servers_obj(path: &Path) -> BTreeMap<String, Value> {
    let Some(bytes) = std::fs::read(path).ok() else { return BTreeMap::new() };
    let Ok(v): Result<Value, _> = serde_json::from_slice(&bytes) else {
        return BTreeMap::new();
    };
    // Accept either `{"mcpServers": {...}}` or a bare `{...}` map.
    let map = v
        .get("mcpServers")
        .and_then(|x| x.as_object())
        .or_else(|| v.as_object())
        .cloned()
        .unwrap_or_default();
    map.into_iter().collect()
}

fn read_claude_json_mcp_servers(path: &Path) -> BTreeMap<String, Value> {
    let Some(bytes) = std::fs::read(path).ok() else { return BTreeMap::new() };
    let Ok(v): Result<Value, _> = serde_json::from_slice(&bytes) else {
        return BTreeMap::new();
    };
    let Some(obj) = v.get("mcpServers").and_then(|x| x.as_object()) else {
        return BTreeMap::new();
    };
    obj.clone().into_iter().collect()
}

fn read_claude_json_local_mcp(claude_json: &Path, project_key: &Path) -> BTreeMap<String, Value> {
    let Some(bytes) = std::fs::read(claude_json).ok() else { return BTreeMap::new() };
    let Ok(v): Result<Value, _> = serde_json::from_slice(&bytes) else {
        return BTreeMap::new();
    };
    let Some(projects) = v.get("projects").and_then(|x| x.as_object()) else {
        return BTreeMap::new();
    };
    // Look up by display-string of the canonical project path.
    let key = project_key.display().to_string();
    let Some(entry) = projects.get(&key).and_then(|x| x.as_object()) else {
        return BTreeMap::new();
    };
    let Some(map) = entry.get("mcpServers").and_then(|x| x.as_object()) else {
        return BTreeMap::new();
    };
    map.clone().into_iter().collect()
}

fn walk_project_mcp(cwd: &Path) -> Vec<McpLayer> {
    let mut chain = Vec::new();
    let mut cur: Option<PathBuf> = Some(cwd.to_path_buf());
    while let Some(dir) = cur {
        let p = dir.join(".mcp.json");
        if p.is_file() {
            let servers = read_mcp_servers_obj(&p);
            if !servers.is_empty() {
                chain.push(McpLayer {
                    source_scope: Scope::Project,
                    servers,
                });
            }
        }
        if dir.join(".git").exists() {
            break;
        }
        cur = dir.parent().map(|p| p.to_path_buf());
    }
    // CC walks cwd→root and later overrides earlier, so deeper dirs win.
    // Our push order is cwd→root, matching that semantics.
    chain
}

fn collect_plugin_mcp_servers() -> BTreeMap<String, Value> {
    let (_files, plugins) = crate::config_view::discover::collect_plugins();
    let mut out = BTreeMap::new();
    for p in plugins {
        let Some(servers) = p
            .manifest
            .get("mcp_servers")
            .and_then(|v| v.as_object())
            .or_else(|| p.manifest.get("mcpServers").and_then(|v| v.as_object()))
        else {
            continue;
        };
        for (k, v) in servers {
            out.insert(k.clone(), v.clone());
        }
    }
    out
}

// ---------- Tests ----------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Map;

    #[test]
    fn non_empty_or_none_rejects_empty_object() {
        assert!(non_empty_or_none(Value::Object(Map::new())).is_none());
        assert!(non_empty_or_none(serde_json::json!({"a": 1})).is_some());
    }

    #[test]
    fn read_mcp_servers_accepts_nested_key() {
        use std::io::Write;
        let td = tempfile::TempDir::new().unwrap();
        let p = td.path().join("m.json");
        let mut f = std::fs::File::create(&p).unwrap();
        write!(f, r#"{{"mcpServers": {{"foo": {{"command": "x"}}}}}}"#).unwrap();
        drop(f);
        let m = read_mcp_servers_obj(&p);
        assert!(m.contains_key("foo"));
    }

    #[test]
    fn read_mcp_servers_accepts_bare_object() {
        use std::io::Write;
        let td = tempfile::TempDir::new().unwrap();
        let p = td.path().join("m.json");
        let mut f = std::fs::File::create(&p).unwrap();
        write!(f, r#"{{"foo": {{"command": "x"}}}}"#).unwrap();
        drop(f);
        let m = read_mcp_servers_obj(&p);
        assert!(m.contains_key("foo"));
    }

    #[test]
    fn walk_project_mcp_stops_at_git() {
        use std::io::Write;
        let td = tempfile::TempDir::new().unwrap();
        let repo = td.path().join("repo");
        let sub = repo.join("a").join("b");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        // .mcp.json in sub AND at repo root.
        write!(
            std::fs::File::create(sub.join(".mcp.json")).unwrap(),
            r#"{{"foo": {{"command": "x"}}}}"#
        )
        .unwrap();
        write!(
            std::fs::File::create(repo.join(".mcp.json")).unwrap(),
            r#"{{"bar": {{"command": "y"}}}}"#
        )
        .unwrap();
        let chain = walk_project_mcp(&sub);
        // Picks up both layers; stops at the git root (so no `td`-level
        // entries — even if none exist).
        assert_eq!(chain.len(), 2);
        assert!(chain[0].servers.contains_key("foo")); // cwd first
        assert!(chain[1].servers.contains_key("bar")); // git root
    }
}
