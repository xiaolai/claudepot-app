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

    // A CC-side adapter that regenerates expected.json from a live CC
    // tree does not exist yet (parity-harness/README.md §4). Say so
    // instead of silently ignoring the variable.
    if std::env::var_os("CLAUDE_SRC").is_some() {
        eprintln!(
            "warning: CLAUDE_SRC is set, but the CC-side adapter is not \
             implemented — the variable is ignored and goldens stay \
             hand-pinned. See parity-harness/README.md §4."
        );
    }

    let repo_root = workspace_root()?;
    let pinned_cc_version = read_pinned_cc_version(&repo_root)?;
    let fixtures_dir = repo_root.join("parity-harness").join("fixtures");
    if !fixtures_dir.is_dir() {
        bail!(
            "parity-harness/fixtures not found at {}",
            fixtures_dir.display()
        );
    }

    let entries: Vec<PathBuf> = std::fs::read_dir(&fixtures_dir)
        .context("read fixtures dir")?
        .collect::<std::io::Result<Vec<_>>>()
        .context("read fixtures dir entry")?
        .into_iter()
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
    let mut matched = 0usize;
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
        matched += 1;
        match run_fixture(fixture, &pinned_cc_version) {
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

    if only.is_some() && matched == 0 {
        bail!("no fixture matched --only filter");
    }

    eprintln!(
        "\n{ok} passed, {} failed (of {} fixtures)",
        failed.len(),
        matched
    );
    if !failed.is_empty() {
        std::process::exit(1);
    }
    Ok(())
}

/// Run one fixture:
/// 1. Check `notes.md` exists and cites the pinned CC version —
///    expected.json is hand-derived from CC source, so every fixture
///    must carry its provenance (file + line refs + version).
/// 2. Read the `input.json` describing the source bundle.
/// 3. Feed it to `effective_settings::compute_raw`.
/// 4. Fail if compute_raw reports an annotated/plain merge divergence —
///    that's a claudepot-core bug the plain-merge backstop would
///    otherwise hide from the harness.
/// 5. Diff the merged output against `expected.json` by key-path.
fn run_fixture(fixture: &Path, pinned_cc_version: &str) -> Result<()> {
    use claudepot_core::config_view::effective_settings;
    use claudepot_core::config_view::policy::PolicySource;

    check_fixture_notes(fixture, pinned_cc_version)?;

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
            .map(|(origin, value)| {
                let parsed = policy_origin_from_str(&origin).ok_or_else(|| {
                    anyhow!(
                        "unknown policy origin {origin:?} — expected one of \
                         remote / mdm_admin / managed_file_composite / hkcu_user"
                    )
                })?;
                Ok::<_, anyhow::Error>(PolicySource {
                    origin: parsed,
                    value,
                })
            })
            .collect::<Result<Vec<_>>>()?,
    };
    // Use compute_raw so parity goldens compare unmasked merge output —
    // CC's loader is upstream of any serialization-time redaction.
    let result = effective_settings::compute_raw(&input_struct);
    if result.merge_divergence {
        bail!(
            "compute_raw reported an annotated/plain merge divergence for \
             this input — the provenance path and the CC-parity merge \
             disagree. This is a claudepot-core bug (provenance::annotate_merge \
             vs merge::merge_layers), not a fixture problem."
        );
    }
    let actual = result.merged;

    let diffs = json_tree_diff(&actual, &expected);
    if !diffs.is_empty() {
        let mut msg = format!("mismatch at {} key-path(s):", diffs.len());
        for d in &diffs {
            msg.push_str("\n  ");
            msg.push_str(d);
        }
        bail!(msg);
    }
    Ok(())
}

/// Every fixture ships a `notes.md` that cites the CC source the
/// expected.json was derived from, including the pinned version as
/// `claude-code@<version>`. This makes a pin bump checkable: bumping
/// `parity-harness/PINNED_CC_VERSION` without re-deriving the fixtures
/// fails the harness instead of silently passing against stale goldens.
fn check_fixture_notes(fixture: &Path, pinned_cc_version: &str) -> Result<()> {
    let notes_path = fixture.join("notes.md");
    let notes = std::fs::read_to_string(&notes_path).with_context(|| {
        format!(
            "read {} — every fixture must ship a notes.md citing the CC \
             source (file + lines + version) its expected.json was derived \
             from. See parity-harness/README.md §2.",
            notes_path.display()
        )
    })?;
    let want = format!("claude-code@{pinned_cc_version}");
    if !notes.contains(&want) {
        bail!(
            "{} does not cite {want} (the version in \
             parity-harness/PINNED_CC_VERSION). If the pin moved, re-derive \
             expected.json against the new CC source and update notes.md.",
            notes_path.display()
        );
    }
    Ok(())
}

/// Read `parity-harness/PINNED_CC_VERSION` — the single machine-readable
/// record of which CC version the goldens were hand-derived from.
fn read_pinned_cc_version(repo_root: &Path) -> Result<String> {
    let p = repo_root.join("parity-harness").join("PINNED_CC_VERSION");
    let s = std::fs::read_to_string(&p).with_context(|| {
        format!(
            "read {} — the harness requires a machine-readable CC version pin",
            p.display()
        )
    })?;
    let v = s.trim().to_string();
    if v.is_empty() {
        bail!(
            "{} is empty — expected a CC version like 2.1.88",
            p.display()
        );
    }
    Ok(v)
}

fn read_json(p: &Path) -> Result<serde_json::Value> {
    let bytes = std::fs::read(p).with_context(|| format!("read {}", p.display()))?;
    let v = serde_json::from_slice(&bytes).with_context(|| format!("parse {}", p.display()))?;
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
        obj.get(k)
            .and_then(|x| if x.is_null() { None } else { Some(x.clone()) })
    };

    let policy = match obj.get("policy") {
        None => Vec::new(),
        Some(serde_json::Value::Array(arr)) => arr
            .iter()
            .enumerate()
            .map(|(idx, el)| {
                let entry = el
                    .as_object()
                    .ok_or_else(|| anyhow!("policy[{idx}] must be an object"))?;
                let origin = entry
                    .get("origin")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow!("policy[{idx}].origin must be a string"))?
                    .to_string();
                let value = match entry.get("value").cloned() {
                    Some(v) if v.is_null() => None,
                    Some(v) => Some(v),
                    None => None,
                };
                Ok((origin, value))
            })
            .collect::<Result<Vec<_>>>()?,
        Some(_) => bail!("policy must be an array"),
    };

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

/// Structural JSON diff: walk both trees and report every diverging
/// key-path with both values. Object key order is ignored; array order
/// is significant (CC's merge preserves it). An empty result means the
/// trees are equal. Path-anchored reporting replaces the old positional
/// line diff, which misaligned every subsequent line after one
/// insertion in the pretty-printed form.
fn json_tree_diff(actual: &serde_json::Value, expected: &serde_json::Value) -> Vec<String> {
    let mut out = Vec::new();
    json_tree_diff_walk("$", actual, expected, &mut out);
    out
}

fn json_tree_diff_walk(
    path: &str,
    actual: &serde_json::Value,
    expected: &serde_json::Value,
    out: &mut Vec<String>,
) {
    use serde_json::Value;
    match (actual, expected) {
        (Value::Object(ao), Value::Object(bo)) => {
            for (k, av) in ao {
                let p = format!("{path}.{k}");
                match bo.get(k) {
                    Some(bv) => json_tree_diff_walk(&p, av, bv, out),
                    None => out.push(format!("{p}: actual = {av}, expected has no key")),
                }
            }
            for (k, bv) in bo {
                if !ao.contains_key(k) {
                    out.push(format!("{path}.{k}: actual has no key, expected = {bv}"));
                }
            }
        }
        (Value::Array(aa), Value::Array(ba)) => {
            if aa.len() != ba.len() {
                out.push(format!(
                    "{path}: array length {} (actual) != {} (expected)",
                    aa.len(),
                    ba.len()
                ));
            }
            for (i, (av, bv)) in aa.iter().zip(ba.iter()).enumerate() {
                json_tree_diff_walk(&format!("{path}[{i}]"), av, bv, out);
            }
        }
        (a, b) => {
            if a != b {
                out.push(format!("{path}: actual = {a}, expected = {b}"));
            }
        }
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
