//! Managed / policy settings — four origins, first-non-empty-valid wins.
//!
//! Per `dev-docs/config-section-plan.md` §8.1 + D16:
//!
//! ```text
//!   remote  →  MDM  →  managed-file-composite  →  HKCU
//! ```
//!
//! - `managed-file-composite` = `managed-settings.json` +
//!   `managed-settings.d/*.json` in [`crate::paths::managed_settings_dir`]
//!   — a system directory, not `~/.claude` — merged alphabetically into
//!   one composite before comparison.
//! - "Empty" means an object with zero keys. Non-empty-but-schema-invalid
//!   is **rejected** (plan §8.1 invalid-non-empty fallthrough) and
//!   fallthrough continues, with the validation error recorded.
//! - For P3 the Remote and HKCU sources are extension points —
//!   `policy_resolve` accepts caller-provided bytes, so a cache layer or
//!   registry reader can slot in without recompiling the resolver.

use crate::config_view::error::ConfigViewError;
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
/// alphabetically. Audit fix for config_view/policy.rs:103: drop-ins
/// now DEEP-MERGE into the composite — nested objects (e.g.
/// `permissions: { deny: [...], allow: [...] }`) combine instead of
/// the drop-in object replacing the base wholesale. The previous
/// shape did `out.insert(k, vv)` at the top level which silently
/// dropped `permissions.deny` rules from the base when a drop-in
/// only set `permissions.allow`.
///
/// Arrays and scalars at any level overwrite (no concatenation) —
/// CC's settings semantics treat arrays as opaque values, so
/// concatenating would surprise users.
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
                merge_value_into(&mut out, &k, vv);
            }
        }
    }
    Value::Object(out)
}

/// Deep-merge a single (key, value) pair into `target`. If both
/// `target[key]` and `value` are objects, recurse; otherwise
/// `value` overwrites whatever was there.
fn merge_value_into(target: &mut serde_json::Map<String, Value>, key: &str, value: Value) {
    match (target.get_mut(key), value) {
        (Some(Value::Object(existing)), Value::Object(incoming)) => {
            for (k, v) in incoming {
                merge_value_into(existing, &k, v);
            }
        }
        (_, value) => {
            target.insert(key.to_string(), value);
        }
    }
}

fn is_non_empty_object(v: &Value) -> bool {
    matches!(v, Value::Object(m) if !m.is_empty())
}

/// Load a single managed-settings JSON file. Returns `None` when the
/// file is missing; a decoded `Value` when present; an error when
/// present-but-malformed.
pub fn load_managed_file(path: &std::path::Path) -> Result<Option<Value>, ConfigViewError> {
    if !path.is_file() {
        return Ok(None);
    }
    let bytes = std::fs::read(path).map_err(|e| ConfigViewError::Io {
        path: path.to_path_buf(),
        source: e,
    })?;
    let v: Value = serde_json::from_slice(&bytes).map_err(|e| ConfigViewError::Parse {
        path: path.to_path_buf(),
        source: e,
    })?;
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
        // CC's drop-in filter: `.json`, and not hidden.
        if !name.ends_with(".json") || name.starts_with('.') {
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

/// The managed-settings file composite in `managed_dir`:
/// `managed-settings.json` plus the `managed-settings.d/` drop-ins,
/// merged. `None` when there is nothing, or nothing but empty objects —
/// the resolver's own notion of an absent source.
pub fn load_managed_composite(managed_dir: &std::path::Path) -> Option<Value> {
    let base = load_managed_file(&managed_dir.join("managed-settings.json"))
        .ok()
        .flatten();
    let drops = scan_managed_dir(&managed_dir.join("managed-settings.d"));
    if base.is_none() && drops.is_empty() {
        return None;
    }
    let composite = build_managed_composite(base.as_ref(), &drops);
    is_non_empty_object(&composite).then_some(composite)
}

/// Is a managed-settings file composite in force in `managed_dir`?
pub fn managed_composite_present(managed_dir: &std::path::Path) -> bool {
    load_managed_composite(managed_dir).is_some()
}

/// Does `managed-mcp.json` at `path` put CC into enterprise MCP
/// lockout?
///
/// CC's test is **presence**, not content (2.1.274: the enterprise
/// loader's `check()` returns its `present` flag). Only a missing file —
/// or, off Windows, a parent directory that cannot be searched — leaves
/// lockout off. An empty `{}` locks out with zero servers, and a file
/// that cannot be read or parsed "keeps exclusive control": CC fails
/// closed, so this does too.
pub fn managed_mcp_present(path: &std::path::Path) -> bool {
    match std::fs::metadata(path) {
        Ok(_) => true,
        Err(e) => match e.kind() {
            std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory => false,
            #[cfg(not(target_os = "windows"))]
            std::io::ErrorKind::PermissionDenied => false,
            _ => true,
        },
    }
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

    #[test]
    fn managed_mcp_lockout_follows_presence_not_content() {
        let dir = tempfile::TempDir::new().unwrap();
        let p = dir.path().join("managed-mcp.json");
        assert!(!managed_mcp_present(&p), "missing file: no lockout");
        for body in ["{}", "not json", r#"{"mcpServers":{}}"#, r#"{"x":1}"#] {
            std::fs::write(&p, body).unwrap();
            assert!(managed_mcp_present(&p), "{body:?} still locks out");
        }
        std::fs::remove_file(&p).unwrap();
        std::fs::create_dir(&p).unwrap();
        assert!(
            managed_mcp_present(&p),
            "not a regular file: CC fails closed"
        );
    }

    #[test]
    fn a_missing_parent_is_not_a_lockout() {
        let dir = tempfile::TempDir::new().unwrap();
        let file = dir.path().join("plain");
        std::fs::write(&file, "x").unwrap();
        // A path "through" a file: ENOTDIR, which CC reads as absent.
        assert!(!managed_mcp_present(&file.join("managed-mcp.json")));
        assert!(!managed_mcp_present(
            &dir.path().join("nope").join("managed-mcp.json")
        ));
    }

    #[test]
    fn hidden_drop_ins_are_skipped() {
        let dir = tempfile::TempDir::new().unwrap();
        std::fs::write(dir.path().join("10-a.json"), r#"{"a":1}"#).unwrap();
        std::fs::write(dir.path().join(".20-b.json"), r#"{"b":1}"#).unwrap();
        let names: Vec<String> = scan_managed_dir(dir.path())
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        assert_eq!(names, vec!["10-a.json".to_string()]);
    }
}
