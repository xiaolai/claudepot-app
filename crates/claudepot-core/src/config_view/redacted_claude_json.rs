//! Redacted view of `~/.claude.json`. Allowlist from plan §7.6 (drawn
//! from the real `GlobalConfig` type at `utils/config.ts:183-440`).
//!
//! Rules:
//! - Explicit allowlist keys: surfaced, with string leaves passed
//!   through `mask::mask_json` so embedded secrets don't escape.
//! - Explicit denylist: never appear, not even summarized.
//! - Everything else: summarized as `[redacted: N internal/telemetry
//!   keys]` with a stable-ordered list of the KEY NAMES (names aren't
//!   sensitive; values are).

use crate::config_view::mask::mask_json;
use serde_json::{Map, Value};

const ALLOW: &[&str] = &[
    "mcpServers",
    "theme",
    "editorMode",
    "diffTool",
    "verbose",
    "autoUpdates",
    "autoCompactEnabled",
    "showTurnDuration",
    "preferredNotifChannel",
    "claudeAiMcpEverConnected",
    "projects",
    "tipsHistory",
];

const DENY: &[&str] = &[
    "primaryApiKey",
    "oauthAccount",
    "customApiKeyResponses",
    "env",
    "apiKeyHelper",
    "userID",
];

const PER_PROJECT_ALLOW: &[&str] = &[
    "mcpServers",
    "hasTrustDialogAccepted",
    "enableAllProjectMcpServers",
    "enabledMcpjsonServers",
    "disabledMcpjsonServers",
];

pub struct Redacted {
    pub allowed: Map<String, Value>,
    /// Sorted key names (alpha) that we collapsed under
    /// `[redacted: N …]`. Never includes denylist keys.
    pub collapsed_keys: Vec<String>,
}

pub fn redact(raw: &Value) -> Redacted {
    let mut allowed = Map::new();
    let mut collapsed = Vec::new();

    let Some(obj) = raw.as_object() else {
        return Redacted { allowed, collapsed_keys: collapsed };
    };
    for (k, v) in obj {
        if DENY.contains(&k.as_str()) {
            continue; // swallowed — not even summarized
        }
        if ALLOW.contains(&k.as_str()) {
            let mut v = v.clone();
            if k == "projects" {
                v = redact_projects(&v);
            }
            mask_json(&mut v);
            allowed.insert(k.clone(), v);
        } else {
            collapsed.push(k.clone());
        }
    }
    collapsed.sort();
    Redacted { allowed, collapsed_keys: collapsed }
}

fn redact_projects(v: &Value) -> Value {
    let Some(obj) = v.as_object() else {
        return v.clone();
    };
    let mut out = Map::new();
    for (path, proj) in obj {
        let mut kept = Map::new();
        if let Some(proj_obj) = proj.as_object() {
            for (k, pv) in proj_obj {
                if PER_PROJECT_ALLOW.contains(&k.as_str()) {
                    kept.insert(k.clone(), pv.clone());
                }
            }
        }
        out.insert(path.clone(), Value::Object(kept));
    }
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn denylist_swallowed() {
        let raw = json!({
            "primaryApiKey": "sk-ant-api01-XXXXXXXXXXXXXXXXXXXXXXXXXXXXXX",
            "theme": "dark",
        });
        let r = redact(&raw);
        assert!(!r.allowed.contains_key("primaryApiKey"));
        assert!(r.collapsed_keys.is_empty());
        assert_eq!(r.allowed.get("theme"), Some(&json!("dark")));
    }

    #[test]
    fn unknown_keys_collapsed_by_name_only() {
        let raw = json!({
            "_internalTelemetryId": "device-123",
            "theme": "light",
        });
        let r = redact(&raw);
        assert_eq!(r.collapsed_keys, vec!["_internalTelemetryId"]);
        assert!(!r.allowed.contains_key("_internalTelemetryId"));
    }

    #[test]
    fn projects_per_project_allowlist() {
        let raw = json!({
            "projects": {
                "/repo": {
                    "hasTrustDialogAccepted": true,
                    "history": [["secret", 1]],
                    "env": {"X": "sk-ant-api01-HEADPHONESTUFFFFFFFFFFFFFF"},
                }
            }
        });
        let r = redact(&raw);
        let p = r.allowed.get("projects").unwrap();
        let only = p.get("/repo").unwrap().as_object().unwrap();
        assert_eq!(only.len(), 1);
        assert!(only.contains_key("hasTrustDialogAccepted"));
    }

    #[test]
    fn string_secrets_in_allowed_keys_masked() {
        let raw = json!({
            "mcpServers": {
                "foo": {"env": {"TOK": "xoxb-1234-5678-AbCdEfGhIjKlMnOpQrStUv"}}
            }
        });
        let r = redact(&raw);
        let s = serde_json::to_string(&r.allowed).unwrap();
        assert!(s.contains("<redacted:slack_bot>"));
        assert!(!s.contains("xoxb-1234-5678"));
    }
}
