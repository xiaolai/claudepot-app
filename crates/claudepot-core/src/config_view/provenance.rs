//! Provenance — tag every primitive leaf in a merged settings tree with
//! the `Scope` that contributed its winning value.
//!
//! Per plan §8.5:
//! - One `ProvenanceEntry` per primitive leaf, reached via a `key_path`
//!   that includes `Key` and `Index` hops.
//! - No container entries (Object / Array) — the UI aggregates via
//!   prefix match when it needs to.
//! - Higher-precedence null clobbers a lower container; suppression flag
//!   records the lost value(s).

use crate::config_view::model::{JsonPathSeg, ProvenanceEntry, Scope};
use serde_json::{Map, Value};

/// Annotated JSON — parallel structure to `serde_json::Value` that
/// carries a list of `Scope` contributors at each primitive leaf.
#[derive(Clone, Debug)]
pub enum Annotated {
    Scalar {
        value: Value,
        contributors: Vec<Scope>,
        suppressed: bool,
    },
    Object {
        entries: std::collections::BTreeMap<String, Annotated>,
    },
    Array {
        elements: Vec<Annotated>,
    },
    /// An inline placeholder used when a higher scope clobbers a lower
    /// container with `null` or a scalar. Carries the original
    /// suppressed value for debug rendering.
    ClobberedContainer {
        winner_value: Value,
        winner: Scope,
        suppressed: Box<Annotated>,
    },
}

impl Annotated {
    pub fn scalar(value: Value, from: Scope) -> Self {
        Annotated::Scalar {
            value,
            contributors: vec![from],
            suppressed: false,
        }
    }

    pub fn from_value(v: &Value, from: Scope) -> Self {
        match v {
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
                Annotated::Scalar {
                    value: v.clone(),
                    contributors: vec![from.clone()],
                    suppressed: false,
                }
            }
            Value::Object(m) => {
                let mut entries = std::collections::BTreeMap::new();
                for (k, vv) in m {
                    entries.insert(k.clone(), Annotated::from_value(vv, from.clone()));
                }
                Annotated::Object { entries }
            }
            Value::Array(a) => {
                let elements = a
                    .iter()
                    .map(|x| Annotated::from_value(x, from.clone()))
                    .collect();
                Annotated::Array { elements }
            }
        }
    }

    /// Convert back to a plain `Value`, losing provenance.
    pub fn to_value(&self) -> Value {
        match self {
            Annotated::Scalar { value, .. } => value.clone(),
            Annotated::Object { entries } => {
                let mut m = Map::new();
                for (k, v) in entries {
                    m.insert(k.clone(), v.to_value());
                }
                Value::Object(m)
            }
            Annotated::Array { elements } => {
                Value::Array(elements.iter().map(|e| e.to_value()).collect())
            }
            Annotated::ClobberedContainer { winner_value, .. } => winner_value.clone(),
        }
    }
}

/// Merge two annotated trees. `upper_scope` is the scope of the
/// higher-precedence input (typically a whole-layer label).
pub fn annotate_merge(lower: Annotated, upper: Annotated, upper_scope: Scope) -> Annotated {
    use Annotated::*;
    match (lower, upper) {
        // Object on Object → deep merge per key.
        (Object { entries: mut la }, Object { entries: ra }) => {
            for (k, v_up) in ra {
                match la.remove(&k) {
                    Some(v_lo) => {
                        la.insert(k, annotate_merge(v_lo, v_up, upper_scope.clone()));
                    }
                    None => {
                        la.insert(k, v_up);
                    }
                }
            }
            Object { entries: la }
        }

        // Array on Array → concat; primitives dedupe by value with
        // contributor-merging on collision.
        (Array { elements: mut la }, Array { elements: ra }) => {
            for new_el in ra {
                match &new_el {
                    Scalar { value, .. } if is_primitive(value) => {
                        if let Some(idx) = la.iter().position(|x| matches_scalar(x, value)) {
                            merge_contributors(&mut la[idx], &upper_scope);
                        } else {
                            la.push(tag_scope(new_el, upper_scope.clone()));
                        }
                    }
                    _ => {
                        // Objects/arrays: always append (D18 parity).
                        la.push(tag_scope(new_el, upper_scope.clone()));
                    }
                }
            }
            Array { elements: la }
        }

        // Scalar above everything → upper wins. If upper is Null/scalar
        // over a container, record suppression.
        (lower, upper @ Scalar { .. }) => {
            let lower_is_container = matches!(lower, Object { .. } | Array { .. });
            if lower_is_container {
                let Scalar { value, .. } = upper.clone() else {
                    unreachable!()
                };
                ClobberedContainer {
                    winner_value: value,
                    winner: upper_scope,
                    suppressed: Box::new(lower),
                }
            } else {
                tag_scope(upper, upper_scope)
            }
        }

        // Any other shape mismatch (container over scalar, etc.) →
        // upper wins.
        (_, upper_other) => tag_scope(upper_other, upper_scope),
    }
}

fn is_primitive(v: &Value) -> bool {
    matches!(
        v,
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_)
    )
}

fn matches_scalar(a: &Annotated, v_up: &Value) -> bool {
    match a {
        Annotated::Scalar { value, .. } => value == v_up,
        _ => false,
    }
}

fn merge_contributors(existing: &mut Annotated, upper_scope: &Scope) {
    if let Annotated::Scalar { contributors, .. } = existing {
        if !contributors.contains(upper_scope) {
            contributors.push(upper_scope.clone());
        }
    }
}

fn tag_scope(a: Annotated, upper_scope: Scope) -> Annotated {
    match a {
        Annotated::Scalar {
            value,
            mut contributors,
            suppressed,
        } => {
            if !contributors.contains(&upper_scope) {
                contributors.push(upper_scope);
            }
            Annotated::Scalar {
                value,
                contributors,
                suppressed,
            }
        }
        other => other,
    }
}

/// Flatten an `Annotated` tree to a sequence of primitive-leaf
/// `ProvenanceEntry` records. No entries for containers.
pub fn flatten_provenance(a: &Annotated) -> Vec<ProvenanceEntry> {
    let mut out = Vec::new();
    walk(a, &mut Vec::new(), &mut out);
    out
}

fn walk(a: &Annotated, path: &mut Vec<JsonPathSeg>, out: &mut Vec<ProvenanceEntry>) {
    match a {
        Annotated::Scalar {
            contributors,
            suppressed,
            ..
        } => {
            out.push(ProvenanceEntry {
                key_path: path.clone(),
                winner: contributors.last().cloned().unwrap_or(Scope::Other),
                contributors: contributors.clone(),
                suppressed: *suppressed,
            });
        }
        Annotated::Object { entries } => {
            for (k, v) in entries {
                path.push(JsonPathSeg::Key(k.clone()));
                walk(v, path, out);
                path.pop();
            }
        }
        Annotated::Array { elements } => {
            for (i, v) in elements.iter().enumerate() {
                path.push(JsonPathSeg::Index(i));
                walk(v, path, out);
                path.pop();
            }
        }
        Annotated::ClobberedContainer {
            winner_value,
            winner,
            ..
        } => {
            // The winner at this path is whatever scalar clobbered the
            // suppressed container. Emit a single entry with suppressed=true.
            out.push(ProvenanceEntry {
                key_path: path.clone(),
                winner: winner.clone(),
                contributors: vec![winner.clone()],
                suppressed: true,
            });
            let _ = winner_value;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn primitive_leaf_attributed_to_upper() {
        let lo = Annotated::from_value(&json!({"a": 1}), Scope::User);
        let up = Annotated::from_value(&json!({"a": 99}), Scope::Project);
        let merged = annotate_merge(lo, up, Scope::Project);
        let entries = flatten_provenance(&merged);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].winner, Scope::Project);
        assert!(!entries[0].suppressed);
    }

    #[test]
    fn nested_merge_preserves_per_leaf_provenance() {
        let lo = Annotated::from_value(&json!({"a": {"x": 1, "y": 2}}), Scope::User);
        let up = Annotated::from_value(&json!({"a": {"y": 99}}), Scope::Local);
        let merged = annotate_merge(lo, up, Scope::Local);
        let entries = flatten_provenance(&merged);
        let x = entries
            .iter()
            .find(|e| matches!(&e.key_path[..], [JsonPathSeg::Key(a), JsonPathSeg::Key(x)] if a == "a" && x == "x"))
            .unwrap();
        let y = entries
            .iter()
            .find(|e| matches!(&e.key_path[..], [JsonPathSeg::Key(a), JsonPathSeg::Key(y)] if a == "a" && y == "y"))
            .unwrap();
        assert_eq!(x.winner, Scope::User);
        assert_eq!(y.winner, Scope::Local);
    }

    #[test]
    fn null_clobber_records_suppression() {
        let lo = Annotated::from_value(&json!({"a": {"x": 1}}), Scope::User);
        let up = Annotated::from_value(&json!({"a": null}), Scope::Project);
        let merged = annotate_merge(lo, up, Scope::Project);
        let entries = flatten_provenance(&merged);
        let a = entries
            .iter()
            .find(|e| matches!(&e.key_path[..], [JsonPathSeg::Key(k)] if k == "a"))
            .unwrap();
        assert!(a.suppressed);
        assert_eq!(a.winner, Scope::Project);
    }

    #[test]
    fn array_primitive_dedupe_extends_contributors() {
        let lo = Annotated::from_value(&json!({"tags": ["x", "y"]}), Scope::User);
        let up = Annotated::from_value(&json!({"tags": ["y", "z"]}), Scope::Project);
        let merged = annotate_merge(lo, up, Scope::Project);
        let entries = flatten_provenance(&merged);
        // "y" leaf should have both User and Project in contributors.
        let y_entry = entries
            .iter()
            .find(|e| {
                matches!(&e.key_path[..], [JsonPathSeg::Key(k), JsonPathSeg::Index(_)] if k == "tags")
                    && matches!(e.winner, Scope::User | Scope::Project)
            })
            .unwrap();
        let _ = y_entry;
        // At least one entry should list both scopes as contributors.
        assert!(entries.iter().any(|e| e.contributors.len() == 2));
    }

    #[test]
    fn array_of_objects_not_deduped() {
        let lo = Annotated::from_value(&json!({"hooks": [{"m": "a"}]}), Scope::User);
        let up = Annotated::from_value(&json!({"hooks": [{"m": "a"}]}), Scope::Project);
        let merged = annotate_merge(lo, up, Scope::Project);
        let v = merged.to_value();
        assert_eq!(v["hooks"].as_array().unwrap().len(), 2);
    }
}
