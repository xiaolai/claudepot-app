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
    effective_mcp::{McpConfigProblem, McpConfigProblemKind, McpLayer, McpSourceBundle},
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
    let (_plugin_files, plugins) = crate::config_view::discover::collect_plugins();
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
        PolicySource {
            origin: PolicyOrigin::Remote,
            value: None,
        },
        PolicySource {
            origin: PolicyOrigin::MdmAdmin,
            value: None,
        },
        PolicySource {
            origin: PolicyOrigin::ManagedFileComposite,
            value: composite,
        },
        PolicySource {
            origin: PolicyOrigin::HkcuUser,
            value: None,
        },
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
    let parsed: Value = serde_json::from_slice(&bytes).ok()?;
    // Settings files MUST be a top-level JSON object. CC's merge
    // customizer (see `merge::merge_settings`) clones the higher-
    // precedence side wholesale when shapes don't match — so a
    // top-level array, scalar, or `null` would clobber every previously
    // merged scope. That is never a legitimate user intent and would
    // silently destroy effective settings; surface a tracing warning
    // and skip the layer instead.
    if !parsed.is_object() {
        tracing::warn!(
            path = %path.display(),
            kind = json_kind(&parsed),
            "settings file is not a top-level JSON object — skipping merge"
        );
        return None;
    }
    Some(parsed)
}

/// Tag for the warning emitted when a settings file is the wrong shape.
fn json_kind(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

/// Resolve CC's global config location.
///
/// Delegates to `paths::global_claude_json_target` rather than
/// re-deriving it. The hand-rolled version this replaces handled
/// `CLAUDE_CONFIG_DIR` but silently dropped the legacy
/// `<claude_config_dir>/.config.json` branch — which is the FIRST
/// thing `getGlobalClaudeFile` (`env.ts:14-26`) checks — while its own
/// comment claimed parity with that function. On a machine carrying
/// the legacy file, the MCP reads below saw an empty map for a file CC
/// was actively applying, and `collect_redacted_user_config` (which
/// did check it) pointed at a different file on the same pane.
fn resolve_claude_json_path() -> PathBuf {
    crate::paths::global_claude_json_target()
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
    //
    // Audit fix for config_view/effective_io.rs:156 — drop entries
    // whose value isn't an object before returning enterprise
    // servers. A managed-mcp.json with malformed entries (e.g. a
    // value that's a bare string or null) used to flow through to
    // CC's startup gate, which then refused to launch with an
    // enterprise-lockout error. Treating malformed entries as
    // absent lets the rest of the bundle merge cleanly; the user
    // can still see the missing entries in the GUI's effective-MCP
    // view because they're omitted, and a later edit fixes the
    // file without an outage.
    let mut problems: Vec<McpConfigProblem> = Vec::new();

    let home = claude_config_dir();
    let (enterprise_raw, enterprise_problem) = read_mcp_servers_obj(&home.join("managed-mcp.json"));
    problems.extend(enterprise_problem);
    let enterprise: BTreeMap<String, Value> = enterprise_raw
        .into_iter()
        .filter(|(_, v)| v.is_object())
        .collect();

    // User: `mcpServers` from ~/.claude.json. The file lives as a
    // sibling of `~/.claude/`, NOT inside it — see
    // `resolve_claude_json_path` for the same logic CC's
    // `getGlobalClaudeFile` (env.ts:14-26) implements.
    let claude_json = resolve_claude_json_path();
    let (user, user_problem) = read_claude_json_mcp_servers(&claude_json);
    problems.extend(user_problem);

    // Local (per-project): ~/.claude.json's
    // `projects[<project-path>].mcpServers`. We use the literal `cwd`
    // as the key — CC canonicalizes via `getProjectPathForConfig`,
    // which we approximate via `find_canonical_git_root`.
    let project_key =
        crate::project_memory::find_canonical_git_root(cwd).unwrap_or_else(|| cwd.to_path_buf());
    let local = read_claude_json_local_mcp(&claude_json, &project_key);

    // Project chain: every `.mcp.json` from cwd up to fs root (or git).
    let (project_chain, chain_problems) = walk_project_mcp(cwd);
    problems.extend(chain_problems);

    // Plugin MCP: each enabled plugin's `manifest.mcp_servers`.
    let plugin = collect_plugin_mcp_servers();

    // Approximation for CC's `isSettingSourceEnabled('projectSettings')`:
    // the source is enabled when the project settings file exists and
    // parses to a non-empty object. CC also disables it via
    // `enabledSettingSources` in the cascade, which we can't read
    // without bootstrap runtime state — the file-presence check is the
    // conservative approximation and matches the observable effect
    // for every Claudepot user today (no SDK embedding).
    let project_settings_path = cwd.join(".claude").join("settings.json");
    let project_settings_enabled = read_settings_file(&project_settings_path)
        .and_then(|v| v.as_object().map(|m| !m.is_empty()))
        .unwrap_or(false);

    McpSourceBundle {
        project_chain,
        user,
        local,
        plugin,
        enterprise,
        effective_settings,
        project_settings_enabled,
        problems,
    }
}

/// Read one JSON file, distinguishing "not there" from "there and
/// broken".
///
/// `Ok(None)` is a missing file — the normal case for `.mcp.json` and
/// `managed-mcp.json`, and never a problem. Everything else that goes
/// wrong produces a [`McpConfigProblem`] instead of an empty map,
/// which is the whole point: an empty map and a failed parse used to
/// be indistinguishable downstream.
fn read_json_file(path: &Path) -> Result<Option<Value>, McpConfigProblem> {
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(McpConfigProblem {
                path: path.to_path_buf(),
                kind: McpConfigProblemKind::Unreadable,
                detail: e.to_string(),
            })
        }
    };
    // CC treats an empty or whitespace-only `.mcp.json` as `{}` rather
    // than as a parse error (`if(!t.trim())return{}` in its own
    // reader), so neither do we.
    if bytes.iter().all(|b| b.is_ascii_whitespace()) {
        return Ok(None);
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|e| McpConfigProblem {
            path: path.to_path_buf(),
            kind: McpConfigProblemKind::MalformedJson,
            // serde's message carries a line/column, which is what makes
            // this actionable. It does not echo the file's contents.
            detail: e.to_string(),
        })
}

/// Read an `.mcp.json`-shaped file: `{"mcpServers": {...}}`.
///
/// There is deliberately **no bare-map fallback**. The old reader fell
/// back to treating the whole root object as the server map when
/// `mcpServers` was absent, which did not model CC — CC's schema is
/// `z.object({ mcpServers: z.record(...).default({}) })` and its error
/// says *"not valid JSON, or mcpServers is not an object"*, so a bare
/// map is never accepted. Worse, the fallback did not merely fail to
/// find servers for a VS Code-style `{"servers": {...}}` file: it
/// **invented** one, a server literally named `servers` whose config
/// was the real server map, while the real server stayed invisible.
fn read_mcp_servers_obj(path: &Path) -> (BTreeMap<String, Value>, Option<McpConfigProblem>) {
    let v = match read_json_file(path) {
        Ok(Some(v)) => v,
        Ok(None) => return (BTreeMap::new(), None),
        Err(p) => return (BTreeMap::new(), Some(p)),
    };
    let Some(root) = v.as_object() else {
        return (
            BTreeMap::new(),
            Some(McpConfigProblem {
                path: path.to_path_buf(),
                kind: McpConfigProblemKind::ServersNotObject,
                detail: format!("top level is {}, expected an object", json_type_name(&v)),
            }),
        );
    };
    match root.get("mcpServers") {
        Some(Value::Object(map)) => (map.clone().into_iter().collect(), None),
        Some(other) => (
            BTreeMap::new(),
            Some(McpConfigProblem {
                path: path.to_path_buf(),
                kind: McpConfigProblemKind::ServersNotObject,
                detail: format!(
                    "`mcpServers` is {}, expected an object",
                    json_type_name(other)
                ),
            }),
        ),
        // No `mcpServers` key. CC silently reads this as zero servers;
        // we say which keys ARE there, because the overwhelmingly
        // likely cause is the VS Code `"servers"` spelling.
        None if !root.is_empty() => {
            let keys: Vec<&str> = root.keys().map(String::as_str).take(5).collect();
            (
                BTreeMap::new(),
                Some(McpConfigProblem {
                    path: path.to_path_buf(),
                    kind: McpConfigProblemKind::MissingServersKey,
                    detail: format!("found {} instead", keys.join(", ")),
                }),
            )
        }
        None => (BTreeMap::new(), None),
    }
}

/// Name a JSON value's type for an error message. Never its contents —
/// an `.mcp.json` carries API keys in `env`.
fn json_type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "an array",
        Value::Object(_) => "an object",
    }
}

/// `mcpServers` from `~/.claude.json`.
///
/// Unlike `.mcp.json`, a missing `mcpServers` key here is entirely
/// normal — that file holds dozens of unrelated keys — so there is no
/// `MissingServersKey` arm. A malformed `~/.claude.json` still gets
/// reported: it is CC's main config, and reading it as "no servers"
/// is the same silent failure.
fn read_claude_json_mcp_servers(
    path: &Path,
) -> (BTreeMap<String, Value>, Option<McpConfigProblem>) {
    let v = match read_json_file(path) {
        Ok(Some(v)) => v,
        Ok(None) => return (BTreeMap::new(), None),
        Err(p) => return (BTreeMap::new(), Some(p)),
    };
    let Some(obj) = v.get("mcpServers").and_then(|x| x.as_object()) else {
        return (BTreeMap::new(), None);
    };
    (obj.clone().into_iter().collect(), None)
}

/// `projects[<key>].mcpServers` from `~/.claude.json`.
///
/// Returns no problem of its own — the same file is read by
/// [`read_claude_json_mcp_servers`], which reports a malformed one
/// once. Reporting it twice would put the same path on screen twice.
fn read_claude_json_local_mcp(claude_json: &Path, project_key: &Path) -> BTreeMap<String, Value> {
    let Ok(Some(v)) = read_json_file(claude_json) else {
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

fn walk_project_mcp(cwd: &Path) -> (Vec<McpLayer>, Vec<McpConfigProblem>) {
    // Walk cwd → root (depth-first from cwd). Push order is cwd first,
    // then parent, ..., so `chain[0]` is the deepest dir. The ingest
    // loop in `effective_mcp::compute` applies layers in order and
    // overwrites per-name, so we must hand it the list **shallowest
    // first** — reverse before returning.
    let mut chain = Vec::new();
    let mut problems = Vec::new();
    let mut cur: Option<PathBuf> = Some(cwd.to_path_buf());
    while let Some(dir) = cur {
        let p = dir.join(".mcp.json");
        if p.is_file() {
            let (servers, problem) = read_mcp_servers_obj(&p);
            problems.extend(problem);
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
    // Reverse → shallowest first, deepest last (deepest wins).
    chain.reverse();
    // Problems stay in walk order (deepest first). They are reported,
    // not merged, so their order carries no precedence meaning.
    (chain, problems)
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

    /// Write `body` to a fresh `.mcp.json` and read it back.
    fn read_mcp_fixture(body: &str) -> (BTreeMap<String, Value>, Option<McpConfigProblem>) {
        use std::io::Write;
        let td = tempfile::TempDir::new().unwrap();
        let p = td.path().join(".mcp.json");
        let mut f = std::fs::File::create(&p).unwrap();
        write!(f, "{body}").unwrap();
        drop(f);
        read_mcp_servers_obj(&p)
    }

    #[test]
    fn read_mcp_servers_accepts_nested_key() {
        let (m, problem) = read_mcp_fixture(r#"{"mcpServers": {"foo": {"command": "x"}}}"#);
        assert!(m.contains_key("foo"));
        assert_eq!(problem, None);
    }

    #[test]
    fn a_vs_code_servers_key_does_not_fabricate_a_server() {
        // This replaces a test that asserted the OPPOSITE — that a bare
        // top-level map is accepted as the server map. That fallback
        // was never CC's behaviour: CC's reader is
        // `z.object({mcpServers: z.record(...).default({})})` and its
        // error reads "not valid JSON, or mcpServers is not an object"
        // (verified in the 2.1.241 binary). What the fallback actually
        // did to the file shape CC's own 2.1.144 changelog names —
        // VS Code's `"servers"` key — was worse than dropping it: it
        // invented a server called `servers` whose config was the real
        // server map, while the real server never appeared anywhere.
        let (m, problem) = read_mcp_fixture(r#"{"servers": {"foo": {"command": "x"}}}"#);
        assert!(
            !m.contains_key("servers"),
            "a top-level key must never be mistaken for a server name"
        );
        assert!(m.is_empty(), "and no server is invented from it either");
        let problem = problem.expect("silence here is the bug being fixed");
        assert_eq!(problem.kind, McpConfigProblemKind::MissingServersKey);
        assert!(
            problem.detail.contains("servers"),
            "the hint has to name the key that IS there: {}",
            problem.detail
        );
    }

    #[test]
    fn malformed_json_is_reported_not_read_as_empty() {
        // The headline defect: unparseable and "no servers" were the
        // same answer, so a broken file rendered as an empty pane.
        let (m, problem) = read_mcp_fixture(r#"{"mcpServers": {"foo":"#);
        assert!(m.is_empty());
        assert_eq!(
            problem.map(|p| p.kind),
            Some(McpConfigProblemKind::MalformedJson)
        );
    }

    #[test]
    fn mcp_servers_of_the_wrong_type_is_reported() {
        for body in [
            r#"{"mcpServers": []}"#,
            r#"{"mcpServers": "nope"}"#,
            r#"{"mcpServers": 3}"#,
            r#"{"mcpServers": null}"#,
        ] {
            let (m, problem) = read_mcp_fixture(body);
            assert!(m.is_empty(), "{body}");
            assert_eq!(
                problem.map(|p| p.kind),
                Some(McpConfigProblemKind::ServersNotObject),
                "{body}"
            );
        }
    }

    #[test]
    fn a_non_object_root_is_reported() {
        let (m, problem) = read_mcp_fixture("[1, 2, 3]");
        assert!(m.is_empty());
        assert_eq!(
            problem.map(|p| p.kind),
            Some(McpConfigProblemKind::ServersNotObject)
        );
    }

    #[test]
    fn absent_and_empty_files_are_not_problems() {
        // CC returns `{}` for both without complaint
        // (`if(!t.trim())return{}`), so neither may raise a warning —
        // a pane that cried wolf on every project without an
        // `.mcp.json` would be worse than the silence it replaced.
        let td = tempfile::TempDir::new().unwrap();
        let missing = td.path().join(".mcp.json");
        let (m, problem) = read_mcp_servers_obj(&missing);
        assert!(m.is_empty());
        assert_eq!(problem, None, "a missing file is the normal case");

        for body in ["", "   ", "\n\t "] {
            let (m, problem) = read_mcp_fixture(body);
            assert!(m.is_empty());
            assert_eq!(problem, None, "empty file {body:?}");
        }

        let (m, problem) = read_mcp_fixture("{}");
        assert!(m.is_empty());
        assert_eq!(problem, None, "an empty object declares no servers");
    }

    #[test]
    fn an_empty_mcp_servers_map_is_not_a_problem() {
        let (m, problem) = read_mcp_fixture(r#"{"mcpServers": {}}"#);
        assert!(m.is_empty());
        assert_eq!(problem, None);
    }

    #[test]
    fn problem_details_never_echo_file_contents() {
        // An `.mcp.json` carries API keys in `env`. The detail line is
        // rendered in the UI, so it must name types and keys, never
        // values.
        let secret = "sk-ant-oat01-DO-NOT-LEAK";
        let body = format!(r#"{{"servers": {{"foo": {{"env": {{"K": "{secret}"}}}}}}}}"#);
        let (_m, problem) = read_mcp_fixture(&body);
        let detail = problem.expect("expected a hint").detail;
        assert!(!detail.contains(secret), "leaked a secret: {detail}");

        let bad = format!(r#"{{"mcpServers": {{"foo": {{"env": {{"K": "{secret}"}}}}}}"#);
        let (_m, problem) = read_mcp_fixture(&bad);
        let detail = problem.expect("expected a parse error").detail;
        assert!(!detail.contains(secret), "leaked a secret: {detail}");
    }

    #[test]
    fn read_settings_file_skips_top_level_array() {
        // A settings file that is a top-level JSON array would clobber
        // the merged object via `merge_settings`'s scalar-vs-object
        // branch. The reader must reject it before merge.
        use std::io::Write;
        let td = tempfile::TempDir::new().unwrap();
        let p = td.path().join("settings.json");
        let mut f = std::fs::File::create(&p).unwrap();
        write!(f, r#"[1, 2, 3]"#).unwrap();
        drop(f);
        assert!(read_settings_file(&p).is_none());
    }

    #[test]
    fn read_settings_file_skips_top_level_scalar() {
        use std::io::Write;
        let td = tempfile::TempDir::new().unwrap();
        let p = td.path().join("settings.json");
        let mut f = std::fs::File::create(&p).unwrap();
        write!(f, r#""just a string""#).unwrap();
        drop(f);
        assert!(read_settings_file(&p).is_none());
    }

    #[test]
    fn read_settings_file_skips_top_level_null() {
        use std::io::Write;
        let td = tempfile::TempDir::new().unwrap();
        let p = td.path().join("settings.json");
        let mut f = std::fs::File::create(&p).unwrap();
        write!(f, "null").unwrap();
        drop(f);
        assert!(read_settings_file(&p).is_none());
    }

    #[test]
    fn read_settings_file_accepts_top_level_object() {
        use std::io::Write;
        let td = tempfile::TempDir::new().unwrap();
        let p = td.path().join("settings.json");
        let mut f = std::fs::File::create(&p).unwrap();
        write!(f, r#"{{"theme":"dark"}}"#).unwrap();
        drop(f);
        let v = read_settings_file(&p).expect("object should pass");
        assert_eq!(v["theme"], serde_json::json!("dark"));
    }

    #[test]
    fn read_settings_file_skips_invalid_json() {
        use std::io::Write;
        let td = tempfile::TempDir::new().unwrap();
        let p = td.path().join("settings.json");
        let mut f = std::fs::File::create(&p).unwrap();
        write!(f, "not json").unwrap();
        drop(f);
        assert!(read_settings_file(&p).is_none());
    }

    #[test]
    fn walk_project_mcp_stops_at_git() {
        use std::io::Write;
        let td = tempfile::TempDir::new().unwrap();
        let repo = td.path().join("repo");
        let sub = repo.join("a").join("b");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        // .mcp.json in sub AND at repo root. Both carry the
        // `mcpServers` wrapper because that is the only shape CC
        // accepts — these fixtures used to be bare maps, which passed
        // solely because the reader had a bare-map fallback that CC
        // never had.
        write!(
            std::fs::File::create(sub.join(".mcp.json")).unwrap(),
            r#"{{"mcpServers": {{"foo": {{"command": "x"}}}}}}"#
        )
        .unwrap();
        write!(
            std::fs::File::create(repo.join(".mcp.json")).unwrap(),
            r#"{{"mcpServers": {{"bar": {{"command": "y"}}}}}}"#
        )
        .unwrap();
        let (chain, problems) = walk_project_mcp(&sub);
        assert!(problems.is_empty(), "well-formed fixtures raise nothing");
        // Picks up both layers; stops at the git root. Ordering is
        // shallowest first so the last-wins ingest in effective_mcp
        // lets deeper dirs (the cwd) override shallower ones.
        assert_eq!(chain.len(), 2);
        assert!(chain[0].servers.contains_key("bar")); // git root first (shallow)
        assert!(chain[1].servers.contains_key("foo")); // cwd last (deepest, wins)
    }
}
