//! Trust gates — the export-time scrubbing and import-time acceptance
//! shape for executable surfaces.
//!
//! See `dev-docs/project-migrate-spec.md` §7.
//!
//! Surfaces and their scrubbing rules:
//!
//!   - **Hooks** (`settings.json:hooks`) — split out at export into
//!     `proposed-hooks.json`. Importer never auto-installs;
//!     `--accept-hooks` is required.
//!   - **MCP servers** with absolute-path commands — flagged
//!     `needs_resolution`. Default disable at import.
//!   - **Plugins** — registry only; cache payload never carried.
//!   - **Statusline scripts** — code; per-script accept gate.
//!   - **CLAUDE.md / agents / skills / commands** — content only;
//!     content-hash summary, user can untick.

use crate::migrate::error::MigrateError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

/// Output of `split_settings`. The two pieces travel separately in the
/// bundle so the importer's trust panel can show hooks distinct from
/// the rest of the user settings.
#[derive(Debug, Clone)]
pub struct SettingsSplit {
    /// `settings.json` minus the `hooks` key.
    pub scrubbed: Value,
    /// The `hooks` block, or `None` if the source had no hooks.
    pub hooks: Option<Value>,
}

/// Split a parsed `settings.json` into scrubbed + hooks. Pure JSON
/// op; no I/O.
pub fn split_settings(parsed: Value) -> SettingsSplit {
    let Value::Object(mut map) = parsed else {
        return SettingsSplit {
            scrubbed: parsed,
            hooks: None,
        };
    };
    let hooks = map.remove("hooks");
    SettingsSplit {
        scrubbed: Value::Object(map),
        hooks,
    }
}

/// One MCP server entry as it appears in `~/.claude.json:mcpServers`
/// (or the project-scoped variant). We only model the fields the
/// trust gate cares about; unknown fields round-trip via
/// `serde_json::Value` so the importer can re-emit the original
/// shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerEntry {
    /// The configured command. May be a bare program name (`bun`,
    /// `node`) or an absolute path. Absolute paths trigger the
    /// needs-resolution flag.
    pub command: String,

    /// Trust-gate annotation written by the export step. Never read
    /// from the source; imported `claude.json` fragments are scrubbed
    /// first, then the importer rewrites this.
    #[serde(default, rename = "_claudepot")]
    pub claudepot_meta: Option<ClaudepotMcpMeta>,

    /// Round-trip the rest of the shape verbatim.
    #[serde(flatten)]
    pub rest: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudepotMcpMeta {
    #[serde(default)]
    pub needs_resolution: bool,

    /// Original `command` value at export time. Carried so the user
    /// can see the path that was scrubbed even after the import-time
    /// disable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_command: Option<String>,
}

/// Walk an `mcpServers` map and flag entries whose `command` is an
/// absolute path. Mutates the map in place. Returns the count of
/// flagged entries.
pub fn scrub_mcp_servers(map: &mut serde_json::Map<String, Value>) -> usize {
    let mut count = 0;
    for (_name, entry) in map.iter_mut() {
        let Value::Object(obj) = entry else { continue };
        let Some(command) = obj.get("command").and_then(|v| v.as_str()).map(|s| s.to_string())
        else {
            continue;
        };
        if !is_absolute_path_command(&command) {
            continue;
        }
        let meta = serde_json::json!({
            "needs_resolution": true,
            "source_command": command
        });
        obj.insert("_claudepot".to_string(), meta);
        count += 1;
    }
    count
}

/// Cheap predicate: does the command look like an absolute filesystem
/// path? Forward slashes (Unix), backslashes (Windows), drive letters,
/// and UNC all count.
fn is_absolute_path_command(cmd: &str) -> bool {
    if cmd.starts_with('/') || cmd.starts_with('\\') {
        return true;
    }
    if cmd.len() >= 3 {
        let bytes = cmd.as_bytes();
        if bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && (bytes[2] == b'/' || bytes[2] == b'\\') {
            return true;
        }
    }
    false
}

/// One trust-gate item surfaced to the user at import time. The
/// adapter (CLI / GUI) decides how to present them; this struct is
/// the wire shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustGateItem {
    pub kind: TrustGateKind,
    /// Human-readable title (e.g. hook command, server name).
    pub title: String,
    /// Sha256 of the surface's exact bytes; anchors provenance.
    pub content_sha256: String,
    /// True when accepting this item is required for the bundle to
    /// import the surface (e.g. `--accept-hooks` for hooks). False
    /// for purely informational items.
    pub default_reject: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TrustGateKind {
    Hook,
    McpAbsolutePath,
    StatuslineScript,
    Plugin,
    GlobalContent,
}

/// Apply Bucket-C scrubbing to a parsed `~/.claude/settings.json` and
/// produce both the scrubbed body and the proposed-hooks block. The
/// returned `proposed_hooks` is `Some` only when the source had any
/// hooks defined (otherwise the importer skips the trust panel).
pub fn scrub_user_settings(parsed: serde_json::Value) -> SettingsSplit {
    split_settings(parsed)
}

/// Apply Bucket-C scrubbing to a parsed `~/.claude/mcpServers.json` (or
/// the `mcpServers` block of `~/.claude.json`). Returns the count of
/// entries marked `needs_resolution`. The input map is mutated in
/// place.
pub fn scrub_mcp_block(value: &mut serde_json::Value) -> usize {
    if let serde_json::Value::Object(map) = value {
        scrub_mcp_servers(map)
    } else {
        0
    }
}

/// Quarantine xattr stripping (macOS only). Stub for non-mac targets.
/// See spec §7.6 — we only strip from JSONL/config files Claudepot
/// owns; user-installable surfaces (statusline, plugins, hooks) keep
/// quarantine.
pub fn strip_quarantine_xattr(_path: &Path) -> Result<(), MigrateError> {
    #[cfg(target_os = "macos")]
    {
        // The real implementation will shell out to
        //   /usr/bin/xattr -d com.apple.quarantine <path>
        // OR call the libc `removexattr` directly. Deferred to v0.1
        // — bundles produced today don't carry quarantine xattr yet
        // (they're written by claudepot, not downloaded), so
        // post-import nag risk is low. Track in spec §7.6.
        let _ = _path;
        return Ok(());
    }
    #[cfg(not(target_os = "macos"))]
    {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_settings_extracts_hooks() {
        let json = serde_json::json!({
            "theme": "dark",
            "hooks": {
                "PreToolUse": [{"matcher": "Bash", "hooks": []}]
            }
        });
        let s = split_settings(json);
        assert!(s.hooks.is_some());
        assert!(s.scrubbed.get("hooks").is_none());
        assert_eq!(s.scrubbed.get("theme").unwrap(), "dark");
    }

    #[test]
    fn split_settings_handles_no_hooks() {
        let json = serde_json::json!({"theme": "light"});
        let s = split_settings(json);
        assert!(s.hooks.is_none());
        assert!(s.scrubbed.get("theme").is_some());
    }

    #[test]
    fn scrub_flags_absolute_unix_path() {
        let mut map = serde_json::Map::new();
        map.insert(
            "x".to_string(),
            serde_json::json!({"command": "/opt/bin/server"}),
        );
        let n = scrub_mcp_servers(&mut map);
        assert_eq!(n, 1);
        let entry = &map["x"];
        assert_eq!(entry["_claudepot"]["needs_resolution"], true);
        assert_eq!(
            entry["_claudepot"]["source_command"].as_str().unwrap(),
            "/opt/bin/server"
        );
    }

    #[test]
    fn scrub_flags_absolute_windows_path() {
        let mut map = serde_json::Map::new();
        map.insert(
            "y".to_string(),
            serde_json::json!({"command": "C:\\bin\\server.exe"}),
        );
        let n = scrub_mcp_servers(&mut map);
        assert_eq!(n, 1);
    }

    #[test]
    fn scrub_does_not_flag_bare_program() {
        let mut map = serde_json::Map::new();
        map.insert(
            "x".to_string(),
            serde_json::json!({"command": "bun", "args": ["x", "server.ts"]}),
        );
        let n = scrub_mcp_servers(&mut map);
        assert_eq!(n, 0);
        assert!(map["x"].get("_claudepot").is_none());
    }

    #[test]
    fn is_absolute_path_command_classifier() {
        assert!(is_absolute_path_command("/opt/bin/x"));
        assert!(is_absolute_path_command(r"C:\bin\x"));
        assert!(is_absolute_path_command(r"\\server\share\x"));
        assert!(!is_absolute_path_command("bun"));
        assert!(!is_absolute_path_command("./relative"));
        assert!(!is_absolute_path_command("../x"));
    }
}
