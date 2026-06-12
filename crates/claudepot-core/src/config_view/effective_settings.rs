//! Effective settings — merge the enabled file-based sources into a
//! single object, with provenance attached to every primitive leaf.
//!
//! Per plan §8.1 the precedence (low → high) is:
//!
//! ```text
//!   PluginBase → User → Project → Local → Flag → Policy
//! ```
//!
//! The CC-parity harness (`cargo xtask verify-cc-parity`,
//! `parity-harness/`) runs [`compute_raw`] against hand-derived CC
//! goldens and fails on any [`EffectiveSettings::merge_divergence`];
//! `merge::tests` and the nested end-to-end cases here cover the
//! same semantics at the unit level.

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
    /// `true` when the provenance-annotated merge disagreed with the
    /// plain CC-parity merge. `merged` always holds the plain (CC-
    /// correct) result, but on divergence `provenance` was flattened
    /// from the annotated tree and may attribute winners that don't
    /// match the merged values. Consumers (the parity harness, the
    /// Config UI) must treat a `true` here as a bug signal in
    /// `provenance::annotate_merge`, not ignore it.
    pub merge_divergence: bool,
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
        (
            Scope::PluginBase,
            input
                .plugin_base
                .clone()
                .unwrap_or_else(|| Value::Object(Map::new())),
        ),
        (
            Scope::User,
            input
                .user
                .clone()
                .unwrap_or_else(|| Value::Object(Map::new())),
        ),
        (
            Scope::Project,
            input
                .project
                .clone()
                .unwrap_or_else(|| Value::Object(Map::new())),
        ),
        (
            Scope::Local,
            input
                .local
                .clone()
                .unwrap_or_else(|| Value::Object(Map::new())),
        ),
        (
            Scope::Flag,
            input
                .flag
                .clone()
                .unwrap_or_else(|| Value::Object(Map::new())),
        ),
    ];

    let policy_layer = policy.effective.clone().map(|p| {
        (
            policy
                .winner
                .clone()
                .map(|o| Scope::Policy { origin: o })
                .unwrap_or(Scope::Other),
            p,
        )
    });

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
    // based view since it's what CC would produce — but we surface the
    // divergence (fail-loud) instead of swapping silently: provenance
    // was built from the annotated tree, so on divergence the winner
    // attributions no longer match the merged values.
    let non_provenance_layers: Vec<Value> = layers
        .iter()
        .map(|(_, l)| l.clone())
        .chain(policy_layer.as_ref().map(|(_, l)| l.clone()))
        .collect();
    let plain = merge::merge_layers(&non_provenance_layers);
    let merge_divergence = !values_equal_ignoring_object_key_order(&merged, &plain);
    if merge_divergence {
        tracing::warn!(
            annotated = %merged,
            plain = %plain,
            "effective-settings merge divergence: annotated merge disagrees \
             with plain CC-parity merge; keeping the plain result, but \
             provenance attributions are unreliable for this input \
             (bug in provenance::annotate_merge or merge::merge_layers)"
        );
        merged = plain;
    }

    let provenance = provenance::flatten_provenance(&merged_annot);

    EffectiveSettings {
        merged,
        provenance,
        policy,
        merge_divergence,
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
        assert!(!r.merge_divergence);
    }

    /// Lock the divergence signal with a REAL annotated-vs-plain
    /// divergence: `Annotated::from_value` keeps intra-layer duplicate
    /// primitives in an array, while the plain merge (and CC's
    /// `uniq([...a, ...b])`, settings.ts:529-531 @ claude-code@2.1.88)
    /// dedupes them when the key collides across layers. So
    /// user `{list:[1,1]}` + project `{list:[2]}` yields `[1,1,2]` on
    /// the annotated path but `[1,2]` on the plain path. compute_raw
    /// must (a) keep the plain (CC-correct) result and (b) report the
    /// divergence instead of swapping silently.
    #[test]
    fn divergence_keeps_plain_merge_and_sets_flag() {
        let input = EffectiveSettingsInput {
            user: Some(json!({"list": [1, 1]})),
            project: Some(json!({"list": [2]})),
            ..Default::default()
        };
        let r = compute_raw(&input);
        // CC-correct output (verified against lodash-es mergeWith +
        // CC's settingsMergeCustomizer): duplicates deduped on merge.
        assert_eq!(r.merged, json!({"list": [1, 2]}));
        assert!(
            r.merge_divergence,
            "annotated/plain divergence must be surfaced, not silently swallowed"
        );
    }

    #[test]
    fn no_divergence_on_plain_precedence_input() {
        let input = EffectiveSettingsInput {
            user: Some(json!({"theme": "light", "list": ["a"]})),
            project: Some(json!({"theme": "dark", "list": ["b"]})),
            ..Default::default()
        };
        let r = compute_raw(&input);
        assert_eq!(r.merged, json!({"theme": "dark", "list": ["a", "b"]}));
        assert!(!r.merge_divergence);
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
