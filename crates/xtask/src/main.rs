//! Workspace automation. Currently one subcommand:
//!
//! ```text
//!   cargo xtask verify-cc-parity
//! ```
//!
//! See `parity-harness/README.md` for the full design.

use anyhow::{anyhow, bail, Context, Result};
use std::path::{Path, PathBuf};

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let cmd = args.next().unwrap_or_default();
    let rest: Vec<String> = args.collect();

    match cmd.as_str() {
        "verify-cc-parity" => verify_cc_parity(&rest),
        "" | "-h" | "--help" | "help" => {
            eprintln!("{}", USAGE);
            Ok(())
        }
        other => {
            eprintln!("{USAGE}\n\nunknown subcommand: {other}");
            std::process::exit(2);
        }
    }
}

const USAGE: &str = "usage: cargo xtask <subcommand>

subcommands:
  verify-cc-parity [--only <name>]    diff Rust merge output against
                                      parity-harness/fixtures/*/expected.json.
                                      Fails loudly on mismatch.

environment variables:
  CLAUDE_SRC    when set, tells the verifier that a CC-side adapter is
                available for regenerating expected.json. See
                parity-harness/README.md §4 for the adapter contract.
";

fn verify_cc_parity(args: &[String]) -> Result<()> {
    let mut only: Option<String> = None;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--only" => {
                only = Some(
                    it.next()
                        .ok_or_else(|| anyhow!("--only needs a fixture name"))?
                        .clone(),
                );
            }
            other => bail!("unknown arg: {other}"),
        }
    }

    let repo_root = workspace_root()?;
    let fixtures_dir = repo_root.join("parity-harness").join("fixtures");
    if !fixtures_dir.is_dir() {
        bail!(
            "parity-harness/fixtures not found at {}",
            fixtures_dir.display()
        );
    }

    let entries: Vec<PathBuf> = std::fs::read_dir(&fixtures_dir)
        .context("read fixtures dir")?
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.path())
        .collect();
    let mut sorted = entries;
    sorted.sort();

    if sorted.is_empty() {
        bail!(
            "no fixtures in {}. Add at least case_01_* to start.",
            fixtures_dir.display()
        );
    }

    let mut ok = 0usize;
    let mut failed: Vec<(String, String)> = Vec::new();
    for fixture in &sorted {
        let name = fixture
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        if let Some(filter) = &only {
            if !name.contains(filter.as_str()) {
                continue;
            }
        }
        match run_fixture(fixture) {
            Ok(()) => {
                eprintln!("✓ {name}");
                ok += 1;
            }
            Err(e) => {
                eprintln!("✗ {name}: {e}");
                failed.push((name, e.to_string()));
            }
        }
    }

    eprintln!("\n{ok} passed, {} failed (of {} fixtures)", failed.len(), sorted.len());
    if !failed.is_empty() {
        std::process::exit(1);
    }
    Ok(())
}

/// Run one fixture:
/// 1. Read the `input.json` describing the source bundle.
/// 2. Feed it to `effective_settings::compute`.
/// 3. Diff the merged output against `expected.json`.
/// 4. Return Ok if they match; detailed error otherwise.
fn run_fixture(fixture: &Path) -> Result<()> {
    use claudepot_core::config_view::effective_settings;
    use claudepot_core::config_view::model::PolicyOrigin;
    use claudepot_core::config_view::policy::PolicySource;

    let input_path = fixture.join("input.json");
    let expected_path = fixture.join("expected.json");
    let input = read_json(&input_path)?;
    let expected = read_json(&expected_path)?;

    let bundle = parse_input(&input)?;
    let input_struct = effective_settings::EffectiveSettingsInput {
        plugin_base: bundle.plugin_base,
        user: bundle.user,
        project: bundle.project,
        local: bundle.local,
        flag: bundle.flag,
        policy_sources: bundle
            .policy
            .into_iter()
            .map(|(origin, value)| PolicySource {
                origin: policy_origin_from_str(&origin).unwrap_or(PolicyOrigin::Remote),
                value,
            })
            .collect(),
    };
    let actual = effective_settings::compute(&input_struct).merged;

    if !json_equal_order_insensitive(&actual, &expected) {
        let diff = simple_diff(&actual, &expected);
        bail!(
            "mismatch:\n  actual   = {}\n  expected = {}\n{}",
            actual,
            expected,
            diff
        );
    }
    Ok(())
}

fn read_json(p: &Path) -> Result<serde_json::Value> {
    let bytes = std::fs::read(p).with_context(|| format!("read {}", p.display()))?;
    let v = serde_json::from_slice(&bytes)
        .with_context(|| format!("parse {}", p.display()))?;
    Ok(v)
}

struct ParsedBundle {
    plugin_base: Option<serde_json::Value>,
    user: Option<serde_json::Value>,
    project: Option<serde_json::Value>,
    local: Option<serde_json::Value>,
    flag: Option<serde_json::Value>,
    policy: Vec<(String, Option<serde_json::Value>)>,
}

/// input.json shape:
///
/// ```json
/// {
///   "plugin_base": {...} | null,
///   "user": {...} | null,
///   "project": {...} | null,
///   "local": {...} | null,
///   "flag": {...} | null,
///   "policy": [
///     {"origin": "remote",                 "value": {...} | null},
///     {"origin": "mdm_admin",              "value": null},
///     {"origin": "managed_file_composite", "value": null},
///     {"origin": "hkcu_user",              "value": null}
///   ]
/// }
/// ```
fn parse_input(v: &serde_json::Value) -> Result<ParsedBundle> {
    let obj = v
        .as_object()
        .ok_or_else(|| anyhow!("input.json top level must be an object"))?;
    let take = |k: &str| -> Option<serde_json::Value> {
        obj.get(k).and_then(|x| if x.is_null() { None } else { Some(x.clone()) })
    };

    let policy = obj
        .get("policy")
        .and_then(|x| x.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|el| {
                    let origin = el.get("origin")?.as_str()?.to_string();
                    let value = el.get("value").cloned();
                    let value = match value {
                        Some(v) if v.is_null() => None,
                        Some(v) => Some(v),
                        None => None,
                    };
                    Some((origin, value))
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    Ok(ParsedBundle {
        plugin_base: take("plugin_base"),
        user: take("user"),
        project: take("project"),
        local: take("local"),
        flag: take("flag"),
        policy,
    })
}

fn policy_origin_from_str(s: &str) -> Option<claudepot_core::config_view::model::PolicyOrigin> {
    use claudepot_core::config_view::model::PolicyOrigin;
    Some(match s {
        "remote" => PolicyOrigin::Remote,
        "mdm_admin" => PolicyOrigin::MdmAdmin,
        "managed_file_composite" => PolicyOrigin::ManagedFileComposite,
        "hkcu_user" => PolicyOrigin::HkcuUser,
        _ => return None,
    })
}

fn json_equal_order_insensitive(a: &serde_json::Value, b: &serde_json::Value) -> bool {
    use serde_json::Value;
    match (a, b) {
        (Value::Object(ao), Value::Object(bo)) => {
            if ao.len() != bo.len() {
                return false;
            }
            for (k, va) in ao {
                match bo.get(k) {
                    Some(vb) if json_equal_order_insensitive(va, vb) => continue,
                    _ => return false,
                }
            }
            true
        }
        (Value::Array(a), Value::Array(b)) => {
            a.len() == b.len()
                && a.iter()
                    .zip(b.iter())
                    .all(|(x, y)| json_equal_order_insensitive(x, y))
        }
        (x, y) => x == y,
    }
}

fn simple_diff(actual: &serde_json::Value, expected: &serde_json::Value) -> String {
    // Line-level diff of the pretty-printed form — enough to pinpoint
    // the divergence without pulling a diff crate.
    let a = serde_json::to_string_pretty(actual).unwrap_or_default();
    let b = serde_json::to_string_pretty(expected).unwrap_or_default();
    let a_lines: Vec<&str> = a.lines().collect();
    let b_lines: Vec<&str> = b.lines().collect();
    let mut out = String::new();
    let max = a_lines.len().max(b_lines.len());
    for i in 0..max {
        let al = a_lines.get(i).copied().unwrap_or("");
        let bl = b_lines.get(i).copied().unwrap_or("");
        if al == bl {
            continue;
        }
        out.push_str(&format!("  L{:03}  actual:   {al}\n", i + 1));
        out.push_str(&format!("  L{:03}  expected: {bl}\n", i + 1));
    }
    if out.is_empty() {
        "  (order-only difference in objects)".to_string()
    } else {
        out
    }
}

fn workspace_root() -> Result<PathBuf> {
    // CARGO_MANIFEST_DIR on xtask points to crates/xtask — walk up two.
    let manifest = std::env::var("CARGO_MANIFEST_DIR")
        .context("CARGO_MANIFEST_DIR not set (run via cargo)")?;
    let p = PathBuf::from(manifest);
    Ok(p.parent()
        .and_then(|p| p.parent())
        .ok_or_else(|| anyhow!("xtask manifest path had no grandparent"))?
        .to_path_buf())
}
