//! Derive the canonical `(kind, artifact_key)` an artifact FILE would
//! be recorded under, given its path.
//!
//! This is the inverse of [`super::extract`]: the extractor reads keys
//! out of session JSONL, this reads them off the filesystem. The two
//! must agree exactly or a join between "installed" and "ever fired"
//! silently misses.
//!
//! # Parity with the TypeScript twin
//!
//! `src/sections/config/artifactKey.ts` implements the same mapping for
//! the Config tree's usage badges. **The two are locked together by
//! `crates/claudepot-core/testdata/artifact-key-vectors.json`** — both
//! run those vectors. Change one, change the other, add a vector. Same
//! contract as `testdata/rate-resolution-vectors.json` (see AGENTS.md
//! "Pricing").
//!
//! # Not every artifact kind is trackable
//!
//! Only skills, agents, and commands have a per-file identity. Hooks
//! are 1:N with files — one `settings.json` declares many — so there is
//! no per-file key to derive, and callers must not treat a hook file's
//! absence from the usage ledger as evidence of anything.
//!
//! # Paths are Windows-aware
//!
//! CC records native separators verbatim, so every marker match and
//! segment split accepts both `/` and `\` (see `.claude/rules/paths.md`).

/// Which artifact kinds have a per-file identity.
const TRACKABLE_KINDS: &[&str] = &["skill", "agent", "command"];

/// A resolved artifact identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactIdentity {
    /// `"skill"` | `"agent"` | `"command"` — matches
    /// [`super::ArtifactKind::as_str`].
    pub kind: &'static str,
    /// The key the JSONL extractor would have written.
    pub artifact_key: String,
    /// Owning plugin, when the path sits under the plugin cache.
    pub plugin_id: Option<String>,
}

fn split_segments(path: &str) -> Vec<&str> {
    path.split(['/', '\\']).collect()
}

/// Pull the plugin id out of
/// `…/plugins/cache/<owner>/<plugin>/<version>/…`.
///
/// Returns `None` when the path isn't under the cache, or when it stops
/// before naming a plugin.
pub fn plugin_id_from_path(path: &str) -> Option<String> {
    // Locate the `plugins<sep>cache<sep>` marker without a regex.
    let segs = split_segments(path);
    let idx = segs
        .windows(2)
        .position(|w| w[0] == "plugins" && w[1] == "cache")?;
    // owner = idx+2, plugin = idx+3
    let plugin = segs.get(idx + 3)?;
    if plugin.is_empty() {
        return None;
    }
    Some((*plugin).to_string())
}

/// Wire prefix every MCP tool name carries.
pub const MCP_TOOL_PREFIX: &str = "mcp__";

/// The artifact key for an MCP tool call, or `None` if `tool_name`
/// isn't an MCP tool.
///
/// The key IS the full wire name. Storing `(server, tool)` separately
/// and re-joining would not round-trip: server names may contain
/// underscores and hyphens — `_hypothesi_tauri-mcp-server` is a real
/// one observed in the wild, and it produces a *triple* underscore
/// after the prefix.
pub fn mcp_artifact_key(tool_name: &str) -> Option<String> {
    if !tool_name.starts_with(MCP_TOOL_PREFIX) {
        return None;
    }
    // Require at least one `__` after the prefix, i.e. a server segment
    // AND a tool segment. `mcp__foo` alone names a server, not a call.
    let rest = &tool_name[MCP_TOOL_PREFIX.len()..];
    if !rest.contains("__") {
        return None;
    }
    Some(tool_name.to_string())
}

/// Server name from an MCP tool name.
///
/// Mirrors Claude Code exactly — `AgentTool.tsx:397-401` does
/// `tool.name.split('__')[1]`. That yields `_hypothesi_tauri-mcp-server`
/// for `mcp___hypothesi_tauri-mcp-server__webview_execute_js`, which is
/// correct: the server name genuinely starts with an underscore.
///
/// A server name containing `__` would mis-parse here, exactly as it
/// does in CC. Parity beats cleverness — diverging would attribute
/// calls differently than the tool that produced them.
pub fn mcp_server_from_tool_name(tool_name: &str) -> Option<String> {
    if !tool_name.starts_with(MCP_TOOL_PREFIX) {
        return None;
    }
    let server = tool_name.split("__").nth(1)?;
    if server.is_empty() {
        return None;
    }
    Some(server.to_string())
}

fn stem_from_md(path: &str) -> Option<String> {
    let last = split_segments(path).pop()?;
    let stem = last.strip_suffix(".md")?;
    if stem.is_empty() {
        return None;
    }
    Some(stem.to_string())
}

/// Skill name. CC supports two layouts:
/// `<root>/skills/<name>/SKILL.md` (canonical) and
/// `<root>/skills/<name>.md` (bare file form).
fn skill_name_from_path(path: &str) -> Option<String> {
    let segs = split_segments(path);
    let last = *segs.last()?;
    if last == "SKILL.md" {
        // `len - 2` must not short-circuit the function. Written as
        // `segs.get(segs.len().wrapping_sub(2))?` it underflowed to
        // `usize::MAX` on a single-segment path, `get` returned None,
        // and `?` bailed out of `skill_name_from_path` entirely —
        // skipping the `stem_from_md` fallback below. The TypeScript
        // twin falls through in that case, so the two disagreed on
        // bare `SKILL.md` (TS: "SKILL", Rust: None).
        //
        // Not reachable from a `FileNode` (those paths are absolute,
        // so there is always a parent segment), but this pair is a
        // parity contract — a divergence anywhere is a defect, and an
        // unreachable one is just a latent one.
        if let Some(parent) = segs.len().checked_sub(2).and_then(|i| segs.get(i)) {
            if !parent.is_empty() {
                return Some((*parent).to_string());
            }
        }
    }
    stem_from_md(path)
}

/// True when `path` sits inside `<project_root>/.claude/`.
///
/// Matching the `.claude` segment specifically — rather than a bare
/// `starts_with(project_root)` — is deliberate, but note it is NOT
/// sufficient on its own: in a global-only scan the backend anchors the
/// tree at the home directory, so `project_root` *is* `~` and
/// `~/.claude/skills/x` would match. Callers running a global scan must
/// pass `None`, not the tree's `project_root`. (`config_view::scan_global`
/// → `assemble_tree(&home, true)`.)
fn is_project_scope(path: &str, project_root: Option<&str>) -> bool {
    let Some(root) = project_root else {
        return false;
    };
    if root.is_empty() {
        return false;
    }
    path.starts_with(&format!("{root}/.claude/")) || path.starts_with(&format!("{root}\\.claude\\"))
}

/// Map an artifact file to the key the usage ledger records it under.
///
/// `kind` is the `config_view` node kind string. `project_root` must be
/// `None` for a global-only scan — see [`is_project_scope`].
pub fn artifact_key_for_path(
    kind: &str,
    path: &str,
    project_root: Option<&str>,
) -> Option<ArtifactIdentity> {
    if !TRACKABLE_KINDS.contains(&kind) {
        return None;
    }
    let plugin_id = plugin_id_from_path(path);
    let project_scope = plugin_id.is_none() && is_project_scope(path, project_root);

    match kind {
        "skill" => {
            let name = skill_name_from_path(path)?;
            let artifact_key = match (&plugin_id, project_scope) {
                (Some(pid), _) => format!("plugin:{pid}:{name}"),
                (None, true) => format!("projectSettings:{name}"),
                (None, false) => format!("userSettings:{name}"),
            };
            Some(ArtifactIdentity {
                kind: "skill",
                artifact_key,
                plugin_id,
            })
        }
        "agent" => {
            let name = stem_from_md(path)?;
            let artifact_key = match &plugin_id {
                Some(pid) => format!("{pid}:{name}"),
                None => name,
            };
            Some(ArtifactIdentity {
                kind: "agent",
                artifact_key,
                plugin_id,
            })
        }
        "command" => {
            let name = stem_from_md(path)?;
            let artifact_key = match &plugin_id {
                Some(pid) => format!("/{pid}:{name}"),
                None => format!("/{name}"),
            };
            Some(ArtifactIdentity {
                kind: "command",
                artifact_key,
                plugin_id,
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    /// Run the shared vectors. The TS twin
    /// (`src/sections/config/artifactKey.test.ts`) runs the same file —
    /// a divergence fails on exactly one side, which is the point.
    #[test]
    fn shared_vectors_match() {
        let raw = include_str!("../../testdata/artifact-key-vectors.json");
        let doc: Value = serde_json::from_str(raw).expect("vectors parse");
        let vectors = doc["vectors"].as_array().expect("vectors array");
        assert!(!vectors.is_empty(), "vector file must not be empty");

        for v in vectors {
            let name = v["name"].as_str().unwrap();
            let kind = v["kind"].as_str().unwrap();
            let path = v["path"].as_str().unwrap();
            let project_root = v["project_root"].as_str();
            let got = artifact_key_for_path(kind, path, project_root);

            match got {
                None => assert!(
                    v["expected"].is_null(),
                    "{name}: expected a key, got none (expected {:?})",
                    v["expected"]
                ),
                Some(id) => {
                    let exp = &v["expected"];
                    assert!(!exp.is_null(), "{name}: expected null, got {id:?}");
                    assert_eq!(id.kind, exp["kind"].as_str().unwrap(), "{name}: kind");
                    assert_eq!(
                        id.artifact_key,
                        exp["artifact_key"].as_str().unwrap(),
                        "{name}: artifact_key"
                    );
                    assert_eq!(
                        id.plugin_id.as_deref(),
                        exp["plugin_id"].as_str(),
                        "{name}: plugin_id"
                    );
                }
            }
        }
    }

    #[test]
    fn global_scan_must_pass_none_not_the_home_anchored_project_root() {
        // The bug this guards: `scan_global()` anchors at the home dir,
        // so passing `project_root` mis-keys every user skill as
        // project-scope and the ledger join misses all of them.
        let path = "/Users/u/.claude/skills/mine/SKILL.md";
        let correct = artifact_key_for_path("skill", path, None).unwrap();
        assert_eq!(correct.artifact_key, "userSettings:mine");

        let wrong = artifact_key_for_path("skill", path, Some("/Users/u")).unwrap();
        assert_eq!(
            wrong.artifact_key, "projectSettings:mine",
            "documents the trap: a home-anchored project_root flips the scope"
        );
    }

    #[test]
    fn mcp_key_is_the_verbatim_wire_name() {
        assert_eq!(
            mcp_artifact_key("mcp__mermaider__validate_syntax").as_deref(),
            Some("mcp__mermaider__validate_syntax")
        );
        // Not an MCP tool.
        assert_eq!(mcp_artifact_key("Bash"), None);
        // Server prefix with no tool segment — names a server, not a call.
        assert_eq!(mcp_artifact_key("mcp__lonely"), None);
    }

    #[test]
    fn mcp_server_extraction_mirrors_cc() {
        // CC: `tool.name.split('__')[1]` (AgentTool.tsx:397-401).
        assert_eq!(
            mcp_server_from_tool_name("mcp__mermaider__validate_syntax").as_deref(),
            Some("mermaider")
        );
        // Real name from this repo's transcripts: THREE underscores
        // after the prefix because the server name starts with one.
        assert_eq!(
            mcp_server_from_tool_name("mcp___hypothesi_tauri-mcp-server__webview_execute_js")
                .as_deref(),
            Some("_hypothesi_tauri-mcp-server")
        );
        // Hyphenated server names are common.
        assert_eq!(
            mcp_server_from_tool_name("mcp__codex-cli__codex").as_deref(),
            Some("codex-cli")
        );
        assert_eq!(mcp_server_from_tool_name("Bash"), None);
    }

    #[test]
    fn plugin_id_requires_owner_and_plugin_segments() {
        assert_eq!(
            plugin_id_from_path("/a/plugins/cache/owner/plug/1.0/skills/x/SKILL.md").as_deref(),
            Some("plug")
        );
        assert_eq!(
            plugin_id_from_path("/a/plugins/cache/owner").as_deref(),
            None
        );
        assert_eq!(plugin_id_from_path("/a/skills/x/SKILL.md").as_deref(), None);
    }
}
