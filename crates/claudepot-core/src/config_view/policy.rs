//! Managed / policy settings — four origins, first-non-empty-valid wins.
//!
//! Per `dev-docs/config-section-plan.md` §8.1 + D16:
//!
//! ```text
//!   remote  →  MDM  →  managed-file-composite  →  HKCU
//! ```
//!
//! - `managed-file-composite` = `~/.claude/managed-settings.json` +
//!   `~/.claude/managed-settings.d/*.json` merged alphabetically into
//!   one composite before comparison.
//! - "Empty" means an object with zero keys. Non-empty-but-schema-invalid
//!   is **rejected** (plan §8.1 invalid-non-empty fallthrough) and
//!   fallthrough continues, with the validation error recorded.
//! - For P3 the Remote and HKCU sources are extension points —
//!   `policy_resolve` accepts caller-provided bytes, so a cache layer or
//!   registry reader can slot in without recompiling the resolver.

use crate::config_view::model::PolicyOrigin;
use serde_json::Value;

#[derive(Clone, Debug)]
pub struct PolicySource {
    pub origin: PolicyOrigin,
    /// `None` when the source doesn't exist on this machine. An empty
    /// object (`Some({})`) is treated as "present but empty" — it does
    /// NOT win, and fallthrough continues.
    pub value: Option<Value>,
}

#[derive(Clone, Debug)]
pub struct PolicyError {
    pub origin: PolicyOrigin,
    pub message: String,
}

#[derive(Clone, Debug)]
pub struct PolicyResolved {
    /// Winning source's merged object, or `None` when all sources empty.
    pub effective: Option<Value>,
    pub winner: Option<PolicyOrigin>,
    /// Errors accumulated from rejected non-empty sources that were
    /// skipped because of validation failures.
    pub errors: Vec<PolicyError>,
}

/// Validate + resolve. `validate` returns `Ok(())` when the candidate
/// passes CC's `SettingsSchema().safeParse`. For P3, callers pass the
/// actual validator (e.g. a thin serde-types shim); we default to
/// "every object with ≥1 key is valid" if the caller passes `None`.
pub fn policy_resolve(
    sources: &[PolicySource],
    validate: Option<&dyn Fn(&Value) -> Result<(), String>>,
) -> PolicyResolved {
    let mut errors: Vec<PolicyError> = Vec::new();
    for src in sources {
        let Some(val) = src.value.as_ref() else {
            continue; // missing — skip
        };
        if !is_non_empty_object(val) {
            continue; // empty — skip without error
        }
        if let Some(v) = validate {
            if let Err(e) = v(val) {
                errors.push(PolicyError {
                    origin: src.origin.clone(),
                    message: e,
                });
                continue;
            }
        }
        return PolicyResolved {
            effective: Some(val.clone()),
            winner: Some(src.origin.clone()),
            errors,
        };
    }
    PolicyResolved {
        effective: None,
        winner: None,
        errors,
    }
}

/// `managed-settings.json` + every `managed-settings.d/*.json` merged
/// alphabetically, top-level shallow merge. The individual files must
/// each be valid JSON objects; malformed entries are skipped with an
/// entry in `issues`.
pub fn build_managed_composite(
    base_json: Option<&Value>,
    drop_in_dir_entries: &[(String, Value)],
) -> Value {
    let mut out = serde_json::Map::new();
    if let Some(Value::Object(m)) = base_json {
        for (k, v) in m {
            out.insert(k.clone(), v.clone());
        }
    }
    let mut sorted: Vec<(String, Value)> = drop_in_dir_entries.to_vec();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    for (_name, v) in sorted {
        if let Value::Object(m) = v {
            for (k, vv) in m {
                out.insert(k, vv);
            }
        }
    }
    Value::Object(out)
}

fn is_non_empty_object(v: &Value) -> bool {
    matches!(v, Value::Object(m) if !m.is_empty())
}

/// Load a single managed-settings JSON file. Returns `None` when the
/// file is missing; a decoded `Value` when present; an error when
/// present-but-malformed.
pub fn load_managed_file(path: &std::path::Path) -> Result<Option<Value>, String> {
    if !path.is_file() {
        return Ok(None);
    }
    let bytes = std::fs::read(path).map_err(|e| format!("read {}: {}", path.display(), e))?;
    let v: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
    Ok(Some(v))
}

/// Scan `managed-settings.d/*.json` into `(filename, parsed)` pairs.
pub fn scan_managed_dir(dir: &std::path::Path) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in rd.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".json") {
            continue;
        }
        let Ok(bytes) = std::fs::read(entry.path()) else {
            continue;
        };
        let Ok(v) = serde_json::from_slice::<Value>(&bytes) else {
            continue;
        };
        out.push((name, v));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn mk(origin: PolicyOrigin, v: Option<Value>) -> PolicySource {
        PolicySource { origin, value: v }
    }

    #[test]
    fn first_non_empty_wins_remote_over_mdm() {
        let sources = vec![
            mk(PolicyOrigin::Remote, Some(json!({"a": 1}))),
            mk(PolicyOrigin::MdmAdmin, Some(json!({"b": 2}))),
        ];
        let r = policy_resolve(&sources, None);
        assert_eq!(r.winner, Some(PolicyOrigin::Remote));
        assert_eq!(r.effective, Some(json!({"a": 1})));
    }

    #[test]
    fn empty_remote_falls_through_to_mdm() {
        let sources = vec![
            mk(PolicyOrigin::Remote, Some(json!({}))),
            mk(PolicyOrigin::MdmAdmin, Some(json!({"b": 2}))),
        ];
        let r = policy_resolve(&sources, None);
        assert_eq!(r.winner, Some(PolicyOrigin::MdmAdmin));
    }

    #[test]
    fn missing_sources_skipped_silently() {
        let sources = vec![
            mk(PolicyOrigin::Remote, None),
            mk(PolicyOrigin::MdmAdmin, None),
            mk(PolicyOrigin::ManagedFileComposite, Some(json!({"k": "v"}))),
        ];
        let r = policy_resolve(&sources, None);
        assert_eq!(r.winner, Some(PolicyOrigin::ManagedFileComposite));
    }

    #[test]
    fn invalid_remote_is_rejected_not_returned() {
        let sources = vec![
            mk(PolicyOrigin::Remote, Some(json!({"bad": true}))),
            mk(PolicyOrigin::MdmAdmin, Some(json!({"good": true}))),
        ];
        let validate = |v: &Value| -> Result<(), String> {
            if v.get("bad").is_some() {
                Err("schema: `bad` is not allowed".to_string())
            } else {
                Ok(())
            }
        };
        let r = policy_resolve(&sources, Some(&validate));
        assert_eq!(r.winner, Some(PolicyOrigin::MdmAdmin));
        assert_eq!(r.errors.len(), 1);
        assert_eq!(r.errors[0].origin, PolicyOrigin::Remote);
        assert!(r.errors[0].message.contains("bad"));
    }

    #[test]
    fn all_empty_yields_no_winner() {
        let sources = vec![
            mk(PolicyOrigin::Remote, Some(json!({}))),
            mk(PolicyOrigin::MdmAdmin, None),
        ];
        let r = policy_resolve(&sources, None);
        assert!(r.winner.is_none());
        assert!(r.effective.is_none());
    }

    #[test]
    fn composite_base_plus_dropins_alphabetical() {
        let base = json!({"a": 1, "b": 2});
        let drops = vec![
            ("z.json".to_string(), json!({"a": 99, "c": 3})),
            ("m.json".to_string(), json!({"b": 5, "d": 4})),
        ];
        // alpha order: m, z — m applies first then z; z.a overwrites base.a.
        let composite = build_managed_composite(Some(&base), &drops);
        let m = composite.as_object().unwrap();
        assert_eq!(m["a"], json!(99)); // z overwrites base
        assert_eq!(m["b"], json!(5)); // m overwrites base, z doesn't touch
        assert_eq!(m["c"], json!(3));
        assert_eq!(m["d"], json!(4));
    }

    #[test]
    fn composite_base_only_when_no_dropins() {
        let base = json!({"a": 1});
        let composite = build_managed_composite(Some(&base), &[]);
        assert_eq!(composite, json!({"a": 1}));
    }
}
