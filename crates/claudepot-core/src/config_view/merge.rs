//! Settings merge — verbatim port of CC's `settings_merge_customizer` +
//! lodash `mergeWith` non-array branch.
//!
//! Rules (plan §8.2 – §8.3):
//! - Arrays: concatenate then dedupe **primitives only**, by value
//!   (canonical JSON form). Objects/arrays-of-objects are never deduped —
//!   matches CC's identity-based `new Set(...)` behavior on freshly
//!   parsed JSON (plan D18).
//! - Non-arrays: higher-precedence scalar (including `null`) overwrites
//!   lower; empty object higher → no-op on lower; missing key higher →
//!   lower retained; deep merge otherwise.
//!
//! No CC runtime is reachable from Rust, so this module relies on the
//! golden fixtures in `tests` for parity.

use serde_json::{Map, Value};
use std::collections::HashSet;

/// Merge `lower` + `upper`, where `upper` has higher precedence.
/// Returns a new Value; inputs are borrowed.
pub fn merge_settings(lower: &Value, upper: &Value) -> Value {
    match (lower, upper) {
        (Value::Object(lo), Value::Object(up)) => {
            Value::Object(merge_objects(lo, up))
        }
        (Value::Array(lo), Value::Array(up)) => {
            Value::Array(merge_arrays(lo, up))
        }
        (_, _) => {
            // Scalar-on-scalar / scalar-on-object / null-clobber:
            // higher wins, including Null.
            upper.clone()
        }
    }
}

/// Merge a sequence of settings layers in precedence order
/// (lowest precedence first).
pub fn merge_layers(layers: &[Value]) -> Value {
    match layers.first() {
        None => Value::Object(Map::new()),
        Some(first) => {
            let mut acc = first.clone();
            for layer in layers.iter().skip(1) {
                acc = merge_settings(&acc, layer);
            }
            acc
        }
    }
}

fn merge_objects(lo: &Map<String, Value>, up: &Map<String, Value>) -> Map<String, Value> {
    let mut out = lo.clone();
    for (k, v) in up {
        match out.remove(k) {
            Some(prev) => out.insert(k.clone(), merge_settings(&prev, v)),
            None => out.insert(k.clone(), v.clone()),
        };
    }
    out
}

fn merge_arrays(a: &[Value], b: &[Value]) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::with_capacity(a.len() + b.len());
    out.extend(a.iter().cloned());
    out.extend(b.iter().cloned());
    uniq_primitives_only(&mut out);
    out
}

/// Per plan D18, CC's `uniq([...])` uses `new Set(...)` which is
/// identity-based for object/array values. Our port only dedupes
/// primitives (by canonical JSON form), matching observed CC behavior.
pub fn uniq_primitives_only(items: &mut Vec<Value>) {
    let mut seen: HashSet<String> = HashSet::new();
    items.retain(|v| match v {
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
            seen.insert(canonical_scalar(v))
        }
        _ => true, // never dedupe objects/arrays
    });
}

fn canonical_scalar(v: &Value) -> String {
    // For scalars, serde_json serialization is canonical enough: numbers
    // keep their original textual form (unless mutated upstream), strings
    // are properly escaped, bool / null are unique.
    v.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // Cases map directly to plan §8.4's 15-case table.

    #[test]
    fn case_01_array_primitives_overlap_dedupes() {
        let r = merge_settings(&json!([1, 2, 3]), &json!([3, 4, 5]));
        assert_eq!(r, json!([1, 2, 3, 4, 5]));
    }

    #[test]
    fn case_02_array_of_objects_no_dedupe() {
        // Two equal-content objects from separate parses → both kept.
        let r = merge_settings(&json!([{"x": 1}]), &json!([{"x": 1}]));
        assert_eq!(r, json!([{"x": 1}, {"x": 1}]));
    }

    #[test]
    fn case_03_higher_null_clobbers_lower_object() {
        let r = merge_settings(&json!({"a": {"x": 1}}), &json!({"a": null}));
        assert_eq!(r, json!({"a": null}));
    }

    #[test]
    fn case_04_higher_scalar_clobbers_lower_object() {
        let r = merge_settings(&json!({"a": {"x": 1}}), &json!({"a": "str"}));
        assert_eq!(r, json!({"a": "str"}));
    }

    #[test]
    fn case_05_higher_empty_array_plus_lower_populated_concat() {
        let r = merge_settings(&json!({"a": [1, 2]}), &json!({"a": []}));
        assert_eq!(r, json!({"a": [1, 2]}));
    }

    #[test]
    fn case_06_higher_empty_object_noop() {
        let r = merge_settings(&json!({"a": {"x": 1}}), &json!({"a": {}}));
        assert_eq!(r, json!({"a": {"x": 1}}));
    }

    #[test]
    fn case_07_higher_missing_key_lower_retained() {
        let r = merge_settings(&json!({"a": 1, "b": 2}), &json!({"a": 99}));
        assert_eq!(r, json!({"a": 99, "b": 2}));
    }

    #[test]
    fn case_08_deeply_nested_recursive_merge() {
        let r = merge_settings(
            &json!({"a": {"b": {"c": {"d": 1}}}}),
            &json!({"a": {"b": {"c": {"e": 2}}}}),
        );
        assert_eq!(r, json!({"a": {"b": {"c": {"d": 1, "e": 2}}}}));
    }

    #[test]
    fn case_09_plugin_base_only_returns_itself() {
        let layers = vec![json!({"plugins": {"agent": {"foo": true}}})];
        let r = merge_layers(&layers);
        assert_eq!(r, layers[0]);
    }

    #[test]
    fn case_10_plugin_base_plus_user_user_wins_on_conflict() {
        let plugin_base = json!({"theme": "dark", "verbose": true});
        let user = json!({"theme": "light"});
        let r = merge_layers(&[plugin_base, user]);
        assert_eq!(r, json!({"theme": "light", "verbose": true}));
    }

    #[test]
    fn case_14_array_of_hook_entries_concatenated_preserve_order() {
        let lo = json!({"hooks": {"PreToolUse": [{"m": "a", "command": "x"}]}});
        let up = json!({"hooks": {"PreToolUse": [{"m": "b", "command": "y"}]}});
        let r = merge_settings(&lo, &up);
        assert_eq!(
            r,
            json!({"hooks": {"PreToolUse": [
                {"m": "a", "command": "x"},
                {"m": "b", "command": "y"},
            ]}})
        );
    }

    #[test]
    fn uniq_primitives_preserves_first_occurrence() {
        let mut v = vec![json!(1), json!(2), json!(1), json!(3), json!(2)];
        uniq_primitives_only(&mut v);
        assert_eq!(v, vec![json!(1), json!(2), json!(3)]);
    }

    #[test]
    fn uniq_primitives_keeps_objects() {
        let mut v = vec![json!({"x": 1}), json!({"x": 1})];
        uniq_primitives_only(&mut v);
        assert_eq!(v.len(), 2);
    }

    #[test]
    fn merge_layers_empty_yields_empty_object() {
        let r = merge_layers(&[]);
        assert_eq!(r, json!({}));
    }

    #[test]
    fn merge_layers_sequential_application() {
        let layers = vec![
            json!({"a": 1, "list": [1]}),
            json!({"b": 2, "list": [2]}),
            json!({"a": 99, "list": [1, 3]}),
        ];
        let r = merge_layers(&layers);
        // a: 1 → 1 → 99 (last wins)
        // b: - → 2 → 2
        // list: [1] + [2] = [1,2] (uniq), then + [1,3] = [1,2,3]
        assert_eq!(r, json!({"a": 99, "b": 2, "list": [1, 2, 3]}));
    }
}
