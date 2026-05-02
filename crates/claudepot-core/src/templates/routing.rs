//! Routing rules — declarative defaults for which route a
//! template should be installed against.
//!
//! Rules are stored as JSON at `~/.claudepot/routing-rules.json`
//! (configurable via `CLAUDEPOT_DATA_DIR`). The Tauri install
//! dialog evaluates rules to suggest a route; the user always
//! gets the final say per-template.
//!
//! Rules apply only at install time. A deleted rule never
//! breaks an already-running automation, because the
//! automation carries its own resolved `route_id`.
//!
//! See `dev-docs/templates-implementation-plan.md` §3.5 / §13.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::fs_utils;
use crate::paths;
use crate::routes::Route;

/// Top-level routing-rules document.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct RoutingRules {
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub rules: Vec<RoutingRule>,
}

fn default_schema_version() -> u32 {
    1
}

/// One routing rule. Empty match = always; first match wins.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoutingRule {
    pub id: String,
    /// JSON key is `match`; the trailing underscore is the
    /// Rust-side workaround for the reserved keyword.
    #[serde(default, rename = "match")]
    pub match_: Match,
    pub use_route: UseRoute,
}

/// What a rule matches on. All fields AND-combined; missing
/// fields don't constrain.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct Match {
    #[serde(default)]
    pub blueprint_privacy: Option<String>,
    #[serde(default)]
    pub blueprint_category: Option<String>,
    #[serde(default)]
    pub blueprint_cost_class: Option<String>,
    /// Match against blueprint id. Plain glob — `*` matches any
    /// chars including `.`.
    #[serde(default)]
    pub blueprint_id_pattern: Option<String>,
    /// Catch-all: an empty `Match` already matches everything,
    /// so this is mostly a self-document for rule authors.
    #[serde(default)]
    pub always: bool,
}

/// What route to use when this rule fires.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UseRoute {
    /// A specific route id.
    Specific { route_id: String },
    /// First local route that meets the template's capability
    /// requirements. Honors the user's
    /// "Prefer local routes" toggle without requiring a
    /// custom rule for it.
    FirstLocalCapable,
    /// The user's primary Anthropic route (or built-in `claude`).
    PrimaryAnthropic,
    /// Cheapest capable route.
    CheapestCapable,
}

#[derive(Debug, Error)]
pub enum RoutingError {
    #[error("io error reading routing rules: {0}")]
    Io(#[from] std::io::Error),
    #[error("malformed routing-rules.json: {0}")]
    Malformed(#[from] serde_json::Error),
}

/// Routing-rules storage handle.
#[derive(Debug, Clone)]
pub struct RoutingStore {
    path: PathBuf,
    rules: RoutingRules,
}

impl RoutingStore {
    pub fn open() -> Result<Self, RoutingError> {
        let path = default_path();
        let rules = if path.exists() {
            let bytes = std::fs::read(&path)?;
            if bytes.is_empty() {
                RoutingRules::default()
            } else {
                serde_json::from_slice(&bytes)?
            }
        } else {
            RoutingRules::default()
        };
        Ok(Self { path, rules })
    }

    pub fn at(path: PathBuf) -> Self {
        Self {
            path,
            rules: RoutingRules::default(),
        }
    }

    pub fn rules(&self) -> &RoutingRules {
        &self.rules
    }

    pub fn replace(&mut self, rules: RoutingRules) {
        self.rules = rules;
    }

    pub fn save(&self) -> Result<(), RoutingError> {
        let bytes = serde_json::to_vec_pretty(&self.rules)?;
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        fs_utils::atomic_write(&self.path, &bytes).map_err(RoutingError::Io)?;
        Ok(())
    }
}

fn default_path() -> PathBuf {
    paths::claudepot_data_dir().join("routing-rules.json")
}

/// Evaluate the rules against a blueprint + the user's routes,
/// returning the suggested route id (or `None` for the default
/// `claude` binary).
///
/// Pure function. Does not access the filesystem.
pub fn evaluate(
    rules: &RoutingRules,
    blueprint_privacy: &str,
    blueprint_category: &str,
    blueprint_cost_class: &str,
    blueprint_id: &str,
    routes: &[&Route],
    is_local: &dyn Fn(&Route) -> bool,
    capabilities_match: &dyn Fn(&Route) -> bool,
) -> Suggestion {
    for rule in &rules.rules {
        if !match_rule(
            &rule.match_,
            blueprint_privacy,
            blueprint_category,
            blueprint_cost_class,
            blueprint_id,
        ) {
            continue;
        }
        match &rule.use_route {
            UseRoute::Specific { route_id } => {
                if let Some(rt) = routes.iter().find(|r| r.id.to_string() == *route_id) {
                    if capabilities_match(rt) {
                        return Suggestion::Route(rt.id.to_string());
                    }
                }
            }
            UseRoute::FirstLocalCapable => {
                if let Some(rt) = routes
                    .iter()
                    .find(|r| is_local(r) && capabilities_match(r))
                {
                    return Suggestion::Route(rt.id.to_string());
                }
            }
            UseRoute::CheapestCapable => {
                // Without per-route cost data, treat local as
                // cheapest, then fall through to the first
                // capable cloud route.
                let pick = routes
                    .iter()
                    .find(|r| is_local(r) && capabilities_match(r))
                    .or_else(|| routes.iter().find(|r| capabilities_match(r)));
                if let Some(rt) = pick {
                    return Suggestion::Route(rt.id.to_string());
                }
            }
            UseRoute::PrimaryAnthropic => {
                return Suggestion::DefaultClaude;
            }
        }
    }
    Suggestion::DefaultClaude
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Suggestion {
    Route(String),
    DefaultClaude,
}

fn match_rule(
    m: &Match,
    privacy: &str,
    category: &str,
    cost: &str,
    bp_id: &str,
) -> bool {
    if let Some(p) = &m.blueprint_privacy {
        if p != privacy {
            return false;
        }
    }
    if let Some(c) = &m.blueprint_category {
        if c != category {
            return false;
        }
    }
    if let Some(c) = &m.blueprint_cost_class {
        if c != cost {
            return false;
        }
    }
    if let Some(pat) = &m.blueprint_id_pattern {
        if !glob_match(bp_id, pat) {
            return false;
        }
    }
    true
}

fn glob_match(haystack: &str, pattern: &str) -> bool {
    let h: Vec<char> = haystack.chars().collect();
    let p: Vec<char> = pattern.chars().collect();
    glob_match_inner(&h, 0, &p, 0)
}

fn glob_match_inner(h: &[char], hi: usize, p: &[char], pi: usize) -> bool {
    let mut hi = hi;
    let mut pi = pi;
    loop {
        if pi >= p.len() {
            return hi >= h.len();
        }
        if p[pi] == '*' {
            for skip in 0..=h.len().saturating_sub(hi) {
                if glob_match_inner(h, hi + skip, p, pi + 1) {
                    return true;
                }
            }
            return false;
        }
        if hi >= h.len() {
            return false;
        }
        if p[pi] != '?' && p[pi] != h[hi] {
            return false;
        }
        hi += 1;
        pi += 1;
    }
}

/// `at` is exposed only for tests, but it's also a useful hook
/// if a future Settings panel needs to load rules from a
/// non-default location. Keep it pub.
pub fn at_path(path: &Path) -> Result<RoutingStore, RoutingError> {
    let mut store = RoutingStore::at(path.to_path_buf());
    if path.exists() {
        let bytes = std::fs::read(path)?;
        if !bytes.is_empty() {
            store.rules = serde_json::from_slice(&bytes)?;
        }
    }
    Ok(store)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routes::{AuthScheme, GatewayConfig, Route, RouteProvider};
    use uuid::Uuid;

    fn route(name: &str, model: &str, base_url: &str) -> Route {
        Route {
            id: Uuid::new_v4(),
            name: name.into(),
            provider: RouteProvider::Gateway(GatewayConfig {
                base_url: base_url.into(),
                api_key: "x".into(),
                auth_scheme: AuthScheme::Bearer,
                enable_tool_search: false,
                use_keychain: false,
            }),
            model: model.into(),
            small_fast_model: None,
            additional_models: vec![],
            wrapper_name: format!("claude-{name}"),
            deployment_organization_uuid: Uuid::new_v4(),
            active_on_desktop: false,
            installed_on_cli: true,
            is_private_cloud: false,
            capabilities_override: None,
        }
    }

    fn is_local(r: &Route) -> bool {
        match &r.provider {
            RouteProvider::Gateway(cfg) => cfg.base_url.contains("127.0.0.1"),
            _ => false,
        }
    }
    fn caps_ok(_: &Route) -> bool {
        true
    }

    #[test]
    fn empty_rules_returns_default() {
        let rules = RoutingRules::default();
        let r1 = route("ollama", "llama", "http://127.0.0.1:11434");
        let routes: Vec<&Route> = vec![&r1];
        let s = evaluate(&rules, "any", "it-health", "trivial", "it.x", &routes, &is_local, &caps_ok);
        assert_eq!(s, Suggestion::DefaultClaude);
    }

    #[test]
    fn local_only_rule_picks_local_route() {
        let rules = RoutingRules {
            schema_version: 1,
            rules: vec![RoutingRule {
                id: "rule-local".into(),
                match_: Match {
                    blueprint_privacy: Some("local".into()),
                    ..Match::default()
                },
                use_route: UseRoute::FirstLocalCapable,
            }],
        };
        let r1 = route("ollama", "llama", "http://127.0.0.1:11434");
        let r2 = route("anthropic", "claude", "https://api.anthropic.com");
        let routes: Vec<&Route> = vec![&r1, &r2];
        let s = evaluate(&rules, "local", "it-health", "trivial", "it.x", &routes, &is_local, &caps_ok);
        match s {
            Suggestion::Route(id) => assert_eq!(id, r1.id.to_string()),
            other => panic!("expected local route, got {other:?}"),
        }
    }

    #[test]
    fn first_match_wins() {
        let r1 = route("ollama", "llama", "http://127.0.0.1:11434");
        let rules = RoutingRules {
            schema_version: 1,
            rules: vec![
                RoutingRule {
                    id: "rule-first".into(),
                    match_: Match {
                        blueprint_id_pattern: Some("it.morning-*".into()),
                        ..Match::default()
                    },
                    use_route: UseRoute::Specific {
                        route_id: r1.id.to_string(),
                    },
                },
                RoutingRule {
                    id: "rule-default".into(),
                    match_: Match::default(),
                    use_route: UseRoute::PrimaryAnthropic,
                },
            ],
        };
        let routes: Vec<&Route> = vec![&r1];
        let s = evaluate(
            &rules,
            "any",
            "it-health",
            "trivial",
            "it.morning-health-check",
            &routes,
            &is_local,
            &caps_ok,
        );
        assert!(matches!(s, Suggestion::Route(id) if id == r1.id.to_string()));
    }

    #[test]
    fn id_pattern_glob_works() {
        assert!(glob_match("it.morning-health", "it.*"));
        assert!(!glob_match("audit.x", "it.*"));
        assert!(glob_match("audit.cache-cleanup", "*.cache-*"));
    }

    #[test]
    fn round_trip_save_load() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("rr.json");
        let rules = RoutingRules {
            schema_version: 1,
            rules: vec![RoutingRule {
                id: "r".into(),
                match_: Match {
                    blueprint_privacy: Some("local".into()),
                    ..Match::default()
                },
                use_route: UseRoute::FirstLocalCapable,
            }],
        };
        let mut store = RoutingStore::at(p.clone());
        store.replace(rules.clone());
        store.save().unwrap();
        let store2 = at_path(&p).unwrap();
        assert_eq!(store2.rules(), &rules);
    }

    #[test]
    fn rule_for_blueprint_id_serializes_with_match_field() {
        // The Rust field is `match_` but the JSON key is `match`
        // (renamed). Verify this round-trips faithfully.
        let json = r#"{
            "schema_version": 1,
            "rules": [
                {
                    "id": "x",
                    "match": { "blueprint_id_pattern": "it.*" },
                    "use_route": { "kind": "primary_anthropic" }
                }
            ]
        }"#;
        let parsed: RoutingRules = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.rules.len(), 1);
        assert_eq!(
            parsed.rules[0].match_.blueprint_id_pattern.as_deref(),
            Some("it.*"),
        );

        // Round-trip back to JSON: key must be `match`, not
        // `match_`.
        let s = serde_json::to_string(&parsed).unwrap();
        assert!(s.contains("\"match\""), "got: {s}");
        assert!(!s.contains("\"match_\""), "got: {s}");
    }
}
