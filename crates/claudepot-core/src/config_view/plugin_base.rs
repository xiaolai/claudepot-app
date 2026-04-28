//! PluginBase layer — the merged, allowlisted settings contribution
//! from every enabled plugin.
//!
//! Per `dev-docs/config-section-plan.md` §6.7 + D23:
//!
//! - Each plugin may contribute a `settings.json` (or inline
//!   `manifest.settings`).
//! - We strip each to the CC allowlist (currently just the `agent`
//!   key).
//! - Cross-plugin merge is **top-level shallow overwrite** — later
//!   plugin's `agent` replaces earlier plugin's whole `agent` value.
//! - The resulting base is then merged INTO the file-based settings
//!   cascade via the standard `merge_settings` deep-merge+concat-uniq
//!   rules.

use serde_json::{Map, Value};
use std::path::{Path, PathBuf};

/// A plugin contribution — derived from a `plugin.json` manifest and
/// optional `settings.json` in the plugin's root.
#[derive(Clone, Debug)]
pub struct Plugin {
    pub id: String,
    /// Absolute path to the plugin root directory.
    pub root: PathBuf,
    /// Plugin manifest (parsed from `plugin.json` or
    /// `.claude-plugin/plugin.json`).
    pub manifest: Value,
    pub enabled: bool,
    /// Optional settings bundle the plugin wants merged into the
    /// cascade. If absent, the plugin's `manifest.settings` is used
    /// instead.
    pub settings: Option<Value>,
    /// Source classification for display (Marketplace / Builtin / Inline).
    pub source: PluginSourceDisplay,
}

#[derive(Clone, Debug)]
pub enum PluginSourceDisplay {
    Marketplace { spec: String },
    Builtin,
    Inline,
}

/// CC's strip-to-allowlist function. Only the `agent` top-level key is
/// permitted in plugin-contributed settings at the time of writing
/// (plan §6.7 / pluginLoader.ts).
pub const SETTINGS_ALLOWLIST: &[&str] = &["agent"];

pub fn strip_to_allowlist(v: &Value, allow: &[&str]) -> Value {
    match v {
        Value::Object(m) => {
            let mut out = Map::new();
            for key in allow {
                if let Some(val) = m.get(*key) {
                    out.insert((*key).to_string(), val.clone());
                }
            }
            Value::Object(out)
        }
        _ => Value::Object(Map::new()),
    }
}

/// Build PluginBase: top-level shallow overwrite (NOT deep merge) across
/// enabled plugins (in discovery order). Caller supplies the discovery
/// order; this function doesn't sort.
pub fn build_plugin_base(plugins: &[Plugin]) -> Value {
    let mut out = Map::new();
    for p in plugins {
        if !p.enabled {
            continue;
        }
        let settings = p
            .settings
            .as_ref()
            .or_else(|| p.manifest.get("settings"))
            .cloned()
            .unwrap_or(Value::Object(Map::new()));
        let stripped = strip_to_allowlist(&settings, SETTINGS_ALLOWLIST);
        if let Value::Object(m) = stripped {
            for (k, v) in m {
                out.insert(k, v); // top-level shallow overwrite
            }
        }
    }
    Value::Object(out)
}

/// Read a plugin manifest from disk. Looks for `plugin.json` or
/// `.claude-plugin/plugin.json`.
pub fn load_plugin_manifest(root: &Path) -> Result<Value, String> {
    let candidates = [
        root.join("plugin.json"),
        root.join(".claude-plugin").join("plugin.json"),
    ];
    for p in &candidates {
        if p.is_file() {
            let bytes = std::fs::read(p).map_err(|e| format!("read {}: {}", p.display(), e))?;
            return serde_json::from_slice(&bytes)
                .map_err(|e| format!("parse {}: {}", p.display(), e));
        }
    }
    Err(format!(
        "no plugin manifest in {} (looked for plugin.json / .claude-plugin/plugin.json)",
        root.display()
    ))
}

/// Load a plugin's optional `settings.json`.
pub fn load_plugin_settings(root: &Path) -> Option<Value> {
    let p = root.join("settings.json");
    if !p.is_file() {
        return None;
    }
    let bytes = std::fs::read(p).ok()?;
    serde_json::from_slice(&bytes).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn plugin(id: &str, settings: Value) -> Plugin {
        Plugin {
            id: id.to_string(),
            root: PathBuf::from(format!("/plugins/{id}")),
            manifest: json!({}),
            enabled: true,
            settings: Some(settings),
            source: PluginSourceDisplay::Builtin,
        }
    }

    #[test]
    fn strips_to_agent_only() {
        let v = json!({
            "agent": {"model": "opus"},
            "theme": "should be dropped",
            "mcpServers": {"should-be-dropped": {}},
        });
        let s = strip_to_allowlist(&v, SETTINGS_ALLOWLIST);
        assert_eq!(s, json!({"agent": {"model": "opus"}}));
    }

    #[test]
    fn top_level_overwrite_not_deep_merge() {
        // Two plugins both contribute `agent.model`; later plugin's
        // entire `agent` wins (not a deep merge).
        let a = plugin("a", json!({"agent": {"model": "opus", "tools": ["x"]}}));
        let b = plugin("b", json!({"agent": {"model": "sonnet"}}));
        let base = build_plugin_base(&[a, b]);
        assert_eq!(base, json!({"agent": {"model": "sonnet"}}));
    }

    #[test]
    fn disabled_plugins_ignored() {
        let mut a = plugin("a", json!({"agent": {"model": "x"}}));
        a.enabled = false;
        let b = plugin("b", json!({"agent": {"model": "y"}}));
        let base = build_plugin_base(&[a, b]);
        assert_eq!(base, json!({"agent": {"model": "y"}}));
    }

    #[test]
    fn empty_plugins_yield_empty_base() {
        let base = build_plugin_base(&[]);
        assert_eq!(base, json!({}));
    }

    #[test]
    fn manifest_settings_used_when_no_external_settings_file() {
        let mut p = plugin("a", json!({}));
        p.settings = None;
        p.manifest = json!({"settings": {"agent": {"model": "mini"}}});
        let base = build_plugin_base(&[p]);
        assert_eq!(base, json!({"agent": {"model": "mini"}}));
    }

    #[test]
    fn non_allowlisted_keys_always_dropped_even_from_manifest() {
        let mut p = plugin("a", json!({}));
        p.settings = None;
        p.manifest = json!({"settings": {"agent": {}, "theme": "x"}});
        let base = build_plugin_base(&[p]);
        assert!(!base.as_object().unwrap().contains_key("theme"));
    }
}
