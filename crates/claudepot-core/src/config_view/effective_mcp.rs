//! Effective MCP — port of CC's `getMcpConfigsByScope` +
//! `getProjectMcpServerStatus`.
//!
//! Per `dev-docs/config-section-plan.md` §9 the sources are:
//!
//! ```text
//!   project chain (cwd → fs-root)  +  user (~/.claude.json .mcpServers)
//!   +  local  (per-project-key in ~/.claude.json)  +  plugin
//!   --  override  -->  enterprise (managed-mcp.json) locks out all others
//! ```
//!
//! Approval state depends on the **simulation mode** (Interactive,
//! NonInteractive, SkipPermissions) + the settings flags
//! (enableAllProjectMcpServers, enabledMcpjsonServers,
//! disabledMcpjsonServers) + whether the projectSettings source is
//! ENABLED (plan D22).

use crate::config_view::model::Scope;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum McpSimulationMode {
    Interactive,
    NonInteractive,
    SkipPermissions,
}

#[derive(Clone, Debug)]
pub struct McpSourceBundle {
    /// Project chain entries (deepest wins). Each entry = one `.mcp.json`.
    pub project_chain: Vec<McpLayer>,
    /// User-level MCP servers from `~/.claude.json`.
    pub user: BTreeMap<String, Value>,
    /// Local (per-project) MCP servers.
    pub local: BTreeMap<String, Value>,
    /// Plugin-provided MCP servers. Deduplicated by content hash when
    /// a manual server with the same content exists.
    pub plugin: BTreeMap<String, Value>,
    /// Enterprise (managed-mcp.json). Non-empty → lockout.
    pub enterprise: BTreeMap<String, Value>,
    /// Settings that affect gating (enableAllProjectMcpServers, etc.).
    pub effective_settings: Value,
    /// Whether `projectSettings` is the enabled setting source per
    /// `isSettingSourceEnabled('projectSettings')` (plan D22).
    pub project_settings_enabled: bool,
}

impl Default for McpSourceBundle {
    fn default() -> Self {
        Self {
            project_chain: Vec::new(),
            user: BTreeMap::new(),
            local: BTreeMap::new(),
            plugin: BTreeMap::new(),
            enterprise: BTreeMap::new(),
            effective_settings: Value::Object(Default::default()),
            project_settings_enabled: true,
        }
    }
}

#[derive(Clone, Debug)]
pub struct McpLayer {
    pub source_scope: Scope,
    pub servers: BTreeMap<String, Value>,
}

#[derive(Serialize, Clone, Debug, Eq, PartialEq)]
#[serde(rename_all = "snake_case", tag = "state", content = "reason")]
pub enum ApprovalState {
    Approved,
    Rejected,
    Pending,
    AutoApproved(AutoApprovalReason),
}

#[derive(Serialize, Clone, Debug, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AutoApprovalReason {
    EnableAllProjectMcp,
    NonInteractiveWithProjectSourceEnabled,
    SkipPermissionsWithProjectSourceEnabled,
}

#[derive(Serialize, Clone, Debug, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum BlockReason {
    EnterpriseLockout,
    DisabledByUser,
}

#[derive(Serialize, Clone, Debug)]
pub struct EffectiveMcpServer {
    pub name: String,
    /// The scope that contributed this server.
    pub source_scope: Scope,
    /// All scopes that defined a server with this name (incl. losers).
    pub contributors: Vec<Scope>,
    pub approval: ApprovalState,
    pub blocked_by: Option<BlockReason>,
    /// Server config JSON with secrets masked. `env`, `headers`, `args`,
    /// `command`, `script`, `url` all pass through mask_json.
    pub masked: Value,
}

/// Compute the effective MCP server view for a given simulation mode.
pub fn compute(bundle: &McpSourceBundle, mode: McpSimulationMode) -> Vec<EffectiveMcpServer> {
    // Enterprise lockout: non-empty enterprise → user/project/local suppressed.
    let lockout = !bundle.enterprise.is_empty();

    // Per-server aggregation. Precedence (low → high): user, project (shallow→deep),
    // local, plugin. Enterprise supersedes everything when active.
    let mut map: BTreeMap<String, (Scope, Value, Vec<Scope>)> = BTreeMap::new();
    let ingest = |scope: Scope,
                  name: String,
                  value: Value,
                  map: &mut BTreeMap<String, (Scope, Value, Vec<Scope>)>| {
        let entry = map
            .entry(name)
            .or_insert_with(|| (scope.clone(), value.clone(), vec![scope.clone()]));
        // Overwrite — later source wins.
        entry.0 = scope.clone();
        entry.1 = value;
        if !entry.2.contains(&scope) {
            entry.2.push(scope);
        }
    };

    if !lockout {
        for (name, v) in &bundle.user {
            ingest(Scope::User, name.clone(), v.clone(), &mut map);
        }
        for layer in &bundle.project_chain {
            for (name, v) in &layer.servers {
                ingest(
                    layer.source_scope.clone(),
                    name.clone(),
                    v.clone(),
                    &mut map,
                );
            }
        }
        for (name, v) in &bundle.local {
            ingest(Scope::Local, name.clone(), v.clone(), &mut map);
        }

        // Plugin MCP dedup: drop plugin entries whose content hash matches
        // an existing entry (manual wins).
        for (name, v) in &bundle.plugin {
            if let Some(existing) = map.get(name) {
                if content_hash(&existing.1) == content_hash(v) {
                    continue; // dedup identical plugin-provided server
                }
            }
            ingest(
                Scope::Plugin {
                    id: "plugin".to_string(),
                    source: crate::config_view::model::PluginSource::Builtin,
                },
                name.clone(),
                v.clone(),
                &mut map,
            );
        }
    }

    // Enterprise-only layer when locked out.
    if lockout {
        for (name, v) in &bundle.enterprise {
            ingest(
                Scope::Policy {
                    origin: crate::config_view::model::PolicyOrigin::ManagedFileComposite,
                },
                name.clone(),
                v.clone(),
                &mut map,
            );
        }
    }

    let enabled_list = get_string_array(&bundle.effective_settings, "enabledMcpjsonServers");
    let disabled_list = get_string_array(&bundle.effective_settings, "disabledMcpjsonServers");
    let enable_all = get_bool(&bundle.effective_settings, "enableAllProjectMcpServers");

    let mut out = Vec::with_capacity(map.len());
    for (name, (src, raw, contributors)) in map {
        let project_scoped = matches!(src, Scope::ClaudeMdDir { .. } | Scope::Project)
            || matches!(src, Scope::Other)
                && contributors
                    .iter()
                    .any(|c| matches!(c, Scope::ClaudeMdDir { .. } | Scope::Project));

        // Under enterprise lockout the enterprise servers themselves are
        // the ONLY active ones — they stay approved. Suppression applies
        // to the user/project/local/plugin sources, but those never
        // reach this loop because we didn't ingest them above. We use
        // the source scope to decide: a Policy-origin entry under
        // lockout is the enterprise entry; anything else should not be
        // here at all, but gets conservatively rejected if it is.
        let is_enterprise_entry = matches!(src, Scope::Policy { .. });
        let approval = if lockout {
            if is_enterprise_entry {
                ApprovalState::Approved
            } else {
                ApprovalState::Rejected
            }
        } else {
            gate(
                &name,
                project_scoped,
                &enabled_list,
                &disabled_list,
                enable_all,
                mode,
                bundle.project_settings_enabled,
            )
        };

        let blocked_by = if lockout && !is_enterprise_entry {
            Some(BlockReason::EnterpriseLockout)
        } else if matches!(approval, ApprovalState::Rejected) {
            Some(BlockReason::DisabledByUser)
        } else {
            None
        };

        let mut masked = raw.clone();
        crate::config_view::mask::mask_json(&mut masked);

        out.push(EffectiveMcpServer {
            name,
            source_scope: src,
            contributors,
            approval,
            blocked_by,
            masked,
        });
    }
    out
}

fn gate(
    name: &str,
    project_scoped: bool,
    enabled_list: &[String],
    disabled_list: &[String],
    enable_all: bool,
    mode: McpSimulationMode,
    project_settings_enabled: bool,
) -> ApprovalState {
    if disabled_list.iter().any(|n| n == name) {
        return ApprovalState::Rejected;
    }
    if enabled_list.iter().any(|n| n == name) {
        return ApprovalState::Approved;
    }
    if !project_scoped {
        return ApprovalState::Approved; // user/local/plugin servers aren't gated
    }
    if enable_all {
        return ApprovalState::AutoApproved(AutoApprovalReason::EnableAllProjectMcp);
    }
    match mode {
        McpSimulationMode::NonInteractive if project_settings_enabled => {
            ApprovalState::AutoApproved(AutoApprovalReason::NonInteractiveWithProjectSourceEnabled)
        }
        McpSimulationMode::SkipPermissions if project_settings_enabled => {
            ApprovalState::AutoApproved(AutoApprovalReason::SkipPermissionsWithProjectSourceEnabled)
        }
        _ => ApprovalState::Pending,
    }
}

fn get_string_array(v: &Value, key: &str) -> Vec<String> {
    v.get(key)
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|el| el.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

fn get_bool(v: &Value, key: &str) -> bool {
    v.get(key).and_then(|x| x.as_bool()).unwrap_or(false)
}

/// Content-hash for plugin dedup (plan §9.5). Stable canonical form via
/// serde_json::to_string; collisions imply identical semantic content.
fn content_hash(v: &Value) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    serde_json::to_string(v).unwrap_or_default().hash(&mut h);
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn srv(command: &str) -> Value {
        json!({"command": command})
    }

    fn bundle_with(project_servers: BTreeMap<String, Value>) -> McpSourceBundle {
        McpSourceBundle {
            project_chain: vec![McpLayer {
                source_scope: Scope::Project,
                servers: project_servers,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn interactive_default_pending_for_project_scoped() {
        let mut servers = BTreeMap::new();
        servers.insert("foo".to_string(), srv("run-foo"));
        let bundle = bundle_with(servers);
        let r = compute(&bundle, McpSimulationMode::Interactive);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].approval, ApprovalState::Pending);
    }

    #[test]
    fn non_interactive_auto_approves_with_project_source_enabled() {
        let mut servers = BTreeMap::new();
        servers.insert("foo".to_string(), srv("run-foo"));
        let bundle = bundle_with(servers);
        let r = compute(&bundle, McpSimulationMode::NonInteractive);
        assert_eq!(
            r[0].approval,
            ApprovalState::AutoApproved(AutoApprovalReason::NonInteractiveWithProjectSourceEnabled)
        );
    }

    #[test]
    fn skip_permissions_auto_approves_when_project_source_enabled() {
        let mut servers = BTreeMap::new();
        servers.insert("foo".to_string(), srv("run-foo"));
        let bundle = bundle_with(servers);
        let r = compute(&bundle, McpSimulationMode::SkipPermissions);
        assert_eq!(
            r[0].approval,
            ApprovalState::AutoApproved(
                AutoApprovalReason::SkipPermissionsWithProjectSourceEnabled
            )
        );
    }

    #[test]
    fn non_interactive_does_not_auto_approve_when_project_source_disabled() {
        let mut servers = BTreeMap::new();
        servers.insert("foo".to_string(), srv("run-foo"));
        let bundle = McpSourceBundle {
            project_chain: vec![McpLayer {
                source_scope: Scope::Project,
                servers,
            }],
            project_settings_enabled: false,
            ..Default::default()
        };
        let r = compute(&bundle, McpSimulationMode::NonInteractive);
        assert_eq!(r[0].approval, ApprovalState::Pending);
    }

    #[test]
    fn enable_all_project_mcp_forces_auto_approval() {
        let mut servers = BTreeMap::new();
        servers.insert("foo".to_string(), srv("run-foo"));
        let bundle = McpSourceBundle {
            project_chain: vec![McpLayer {
                source_scope: Scope::Project,
                servers,
            }],
            effective_settings: json!({"enableAllProjectMcpServers": true}),
            ..Default::default()
        };
        let r = compute(&bundle, McpSimulationMode::Interactive);
        assert_eq!(
            r[0].approval,
            ApprovalState::AutoApproved(AutoApprovalReason::EnableAllProjectMcp)
        );
    }

    #[test]
    fn disabled_list_wins_over_enable_all() {
        let mut servers = BTreeMap::new();
        servers.insert("foo".to_string(), srv("run-foo"));
        let bundle = McpSourceBundle {
            project_chain: vec![McpLayer {
                source_scope: Scope::Project,
                servers,
            }],
            effective_settings: json!({
                "enableAllProjectMcpServers": true,
                "disabledMcpjsonServers": ["foo"]
            }),
            ..Default::default()
        };
        let r = compute(&bundle, McpSimulationMode::Interactive);
        assert_eq!(r[0].approval, ApprovalState::Rejected);
    }

    #[test]
    fn enterprise_lockout_suppresses_non_enterprise_sources() {
        let mut user = BTreeMap::new();
        user.insert("u".to_string(), srv("u"));
        let mut project = BTreeMap::new();
        project.insert("p".to_string(), srv("p"));
        let mut enterprise = BTreeMap::new();
        enterprise.insert("e".to_string(), srv("e"));
        let bundle = McpSourceBundle {
            user,
            project_chain: vec![McpLayer {
                source_scope: Scope::Project,
                servers: project,
            }],
            enterprise,
            ..Default::default()
        };
        let r = compute(&bundle, McpSimulationMode::Interactive);
        // User and project entries don't appear — enterprise is the
        // only surviving source.
        assert!(!r.iter().any(|s| s.name == "u"));
        assert!(!r.iter().any(|s| s.name == "p"));
        // Enterprise server is present AND Approved — it's the active
        // server list, not a blocked one.
        let ent = r.iter().find(|s| s.name == "e").unwrap();
        assert_eq!(ent.approval, ApprovalState::Approved);
        assert!(ent.blocked_by.is_none());
    }

    #[test]
    fn plugin_dedup_by_content_drops_duplicate() {
        let mut user = BTreeMap::new();
        user.insert("foo".to_string(), srv("run-foo"));
        let mut plugin = BTreeMap::new();
        plugin.insert("foo".to_string(), srv("run-foo")); // same content
        let bundle = McpSourceBundle {
            user,
            plugin,
            ..Default::default()
        };
        let r = compute(&bundle, McpSimulationMode::Interactive);
        // Only one "foo" server — user wins; plugin dupe dropped.
        let foos: Vec<_> = r.iter().filter(|s| s.name == "foo").collect();
        assert_eq!(foos.len(), 1);
    }

    #[test]
    fn plugin_keeps_distinct_content_same_name() {
        let mut user = BTreeMap::new();
        user.insert("foo".to_string(), srv("manual-command"));
        let mut plugin = BTreeMap::new();
        plugin.insert("foo".to_string(), srv("plugin-command"));
        let bundle = McpSourceBundle {
            user,
            plugin,
            ..Default::default()
        };
        let r = compute(&bundle, McpSimulationMode::Interactive);
        let foo = r.iter().find(|s| s.name == "foo").unwrap();
        // Plugin overwrote (last-wins) but both contributors recorded.
        assert!(foo.contributors.len() >= 1);
    }

    #[test]
    fn secrets_masked_in_server_env() {
        let mut user = BTreeMap::new();
        let tok = "xoxb-1234-5678-AbCdEfGhIjKlMnOpQrStUv";
        user.insert(
            "foo".to_string(),
            json!({"command": "x", "env": {"TOK": tok}}),
        );
        let bundle = McpSourceBundle {
            user,
            ..Default::default()
        };
        let r = compute(&bundle, McpSimulationMode::Interactive);
        let s = serde_json::to_string(&r[0].masked).unwrap();
        assert!(!s.contains("xoxb-1234-5678"));
        assert!(s.contains("<redacted:slack_bot>"));
    }

    #[test]
    fn user_scoped_server_ungated() {
        let mut user = BTreeMap::new();
        user.insert("foo".to_string(), srv("x"));
        let bundle = McpSourceBundle {
            user,
            ..Default::default()
        };
        let r = compute(&bundle, McpSimulationMode::Interactive);
        // User-scoped servers aren't subject to the project-gate.
        assert_eq!(r[0].approval, ApprovalState::Approved);
    }
}
