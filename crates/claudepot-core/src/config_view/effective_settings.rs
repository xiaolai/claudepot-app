//! Effective settings — merge the enabled file-based sources into a
//! single object, with provenance attached to every primitive leaf.
//!
//! Per plan §8.1 the precedence (low → high) is:
//!
//! ```text
//!   PluginBase → User → Project → Local → Flag → Policy
//! ```
//!
//! The CC-parity harness is deferred (plan §8.6); P4 ships hand-
//! authored golden fixtures inside `merge::tests` and nested
//! end-to-end cases here.

use crate::config_view::{
    mask::mask_json,
    merge,
    model::{ProvenanceEntry, Scope},
    policy::{policy_resolve, PolicyResolved, PolicySource},
    provenance::{self, annotate_merge, Annotated},
};
use serde_json::{Map, Value};

/// Input bundle for effective-settings computation. All sources are
/// optional; missing sources contribute nothing.
#[derive(Clone, Debug, Default)]
pub struct EffectiveSettingsInput {
    pub plugin_base: Option<Value>,
    pub user: Option<Value>,
    pub project: Option<Value>,
    pub local: Option<Value>,
    pub flag: Option<Value>,
    pub policy_sources: Vec<PolicySource>,
}

#[derive(Clone, Debug)]
pub struct EffectiveSettings {
    /// Raw merged settings with secrets masked.
    pub merged: Value,
    pub provenance: Vec<ProvenanceEntry>,
    pub policy: PolicyResolved,
}

pub fn compute(input: &EffectiveSettingsInput) -> EffectiveSettings {
    let mut r = compute_raw(input);
    mask_json(&mut r.merged);
    r
}

/// Same as [`compute`] but skips the secret-mask pipeline. Used by the
/// parity harness (`cargo xtask verify-cc-parity`) so goldens reflect
/// CC's upstream merge output rather than a Claudepot-masked view —
/// mask and merge are tested independently, and comparing CC's own
/// pre-serialization JSON is the correct apples-to-apples check. Do
/// NOT expose this output across the IPC boundary.
pub fn compute_raw(input: &EffectiveSettingsInput) -> EffectiveSettings {
    let policy = policy_resolve(&input.policy_sources, None);

    let layers: Vec<(Scope, Value)> = vec![
        (Scope::PluginBase, input.plugin_base.clone().unwrap_or_else(|| Value::Object(Map::new()))),
        (Scope::User, input.user.clone().unwrap_or_else(|| Value::Object(Map::new()))),
        (Scope::Project, input.project.clone().unwrap_or_else(|| Value::Object(Map::new()))),
        (Scope::Local, input.local.clone().unwrap_or_else(|| Value::Object(Map::new()))),
        (Scope::Flag, input.flag.clone().unwrap_or_else(|| Value::Object(Map::new()))),
    ];

    let policy_layer = policy.effective.clone().map(|p| (
        policy.winner.clone().map(|o| Scope::Policy { origin: o }).unwrap_or(Scope::Other),
        p,
    ));

    // Build Annotated tree in precedence order.
    let mut annotated: Option<Annotated> = None;
    for (scope, layer) in &layers {
        if is_empty_object(layer) {
            continue;
        }
        let next = Annotated::from_value(layer, scope.clone());
        annotated = Some(match annotated {
            None => next,
            Some(prev) => annotate_merge(prev, next, scope.clone()),
        });
    }
    if let Some((scope, layer)) = &policy_layer {
        let next = Annotated::from_value(layer, scope.clone());
        annotated = Some(match annotated {
            None => next,
            Some(prev) => annotate_merge(prev, next, scope.clone()),
        });
    }

    let merged_annot = annotated.unwrap_or(Annotated::Object {
        entries: std::collections::BTreeMap::new(),
    });
    let mut merged = merged_annot.to_value();

    // Sanity-check against the straightforward merge (§8.4). If they
    // diverge this is a bug in provenance or merge; we keep the merge-
    // based view since it's what CC would produce.
    let non_provenance_layers: Vec<Value> = layers
        .iter()
        .map(|(_, l)| l.clone())
        .chain(policy_layer.as_ref().map(|(_, l)| l.clone()))
        .collect();
    let plain = merge::merge_layers(&non_provenance_layers);
    if !values_equal_ignoring_object_key_order(&merged, &plain) {
        merged = plain;
    }

    let provenance = provenance::flatten_provenance(&merged_annot);

    EffectiveSettings {
        merged,
        provenance,
        policy,
    }
}

fn is_empty_object(v: &Value) -> bool {
    matches!(v, Value::Object(m) if m.is_empty())
}

fn values_equal_ignoring_object_key_order(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Object(ao), Value::Object(bo)) => {
            if ao.len() != bo.len() {
                return false;
            }
            for (k, va) in ao {
                match bo.get(k) {
                    Some(vb) if values_equal_ignoring_object_key_order(va, vb) => continue,
                    _ => return false,
                }
            }
            true
        }
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len()
                && a.iter()
                    .zip(b.iter())
                    .all(|(x, y)| values_equal_ignoring_object_key_order(x, y))
        }
        (x, y) => x == y,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn empty_input_yields_empty_merged() {
        let r = compute(&EffectiveSettingsInput::default());
        assert_eq!(r.merged, json!({}));
        assert!(r.provenance.is_empty());
        assert!(r.policy.winner.is_none());
    }

    #[test]
    fn precedence_policy_over_local_over_project_over_user_over_plugin() {
        let input = EffectiveSettingsInput {
            plugin_base: Some(json!({"theme": "plugin-theme", "verbose": true})),
            user: Some(json!({"theme": "user-theme"})),
            project: Some(json!({"theme": "project-theme", "diffTool": "p"})),
            local: Some(json!({"theme": "local-theme"})),
            flag: None,
            policy_sources: vec![PolicySource {
                origin: crate::config_view::model::PolicyOrigin::ManagedFileComposite,
                value: Some(json!({"theme": "policy-theme"})),
            }],
        };
        let r = compute(&input);
        assert_eq!(r.merged["theme"], json!("policy-theme"));
        assert_eq!(r.merged["diffTool"], json!("p"));
        assert_eq!(r.merged["verbose"], json!(true));
        assert!(r.policy.winner.is_some());
    }

    #[test]
    fn secrets_masked_in_effective_output() {
        let input = EffectiveSettingsInput {
            user: Some(json!({
                "mcpServers": {
                    "foo": {"env": {"TOK": "xoxb-1234-5678-AbCdEfGhIjKlMnOpQrStUv"}}
                }
            })),
            ..Default::default()
        };
        let r = compute(&input);
        let s = r.merged.to_string();
        assert!(s.contains("<redacted:slack_bot>"));
        assert!(!s.contains("xoxb-1234-5678"));
    }

    #[test]
    fn provenance_attributes_top_level_scalar_to_project() {
        let input = EffectiveSettingsInput {
            user: Some(json!({"theme": "light"})),
            project: Some(json!({"theme": "dark"})),
            ..Default::default()
        };
        let r = compute(&input);
        let winner = r
            .provenance
            .iter()
            .find(|e| matches!(&e.key_path[..], [crate::config_view::model::JsonPathSeg::Key(k)] if k == "theme"))
            .unwrap();
        assert!(matches!(winner.winner, Scope::Project));
    }
}
