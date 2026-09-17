//! Workspace automation. Currently one subcommand:
//!
//! ```text
//!   cargo xtask verify-cc-parity
//!   cargo xtask verify-docs
//! ```
//!
//! See `parity-harness/README.md` for the full design.

mod cc_drift;
mod data_dir_scan;
mod screenshot_fixture;
mod verify_docs;

use anyhow::{anyhow, bail, Context, Result};
use std::path::{Path, PathBuf};

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let cmd = args.next().unwrap_or_default();
    let rest: Vec<String> = args.collect();

    match cmd.as_str() {
        "verify-cc-parity" => verify_cc_parity(&rest),
        "cc-drift" => cc_drift::run(&workspace_root()?, &rest),
        "verify-docs" => verify_docs::verify_docs(&workspace_root()?),
        "verify-screenshots" => verify_docs::verify_screenshots(&workspace_root()?),
        "screenshot-fixture" => {
            let out = rest
                .iter()
                .position(|a| a == "--out")
                .and_then(|i| rest.get(i + 1));
            screenshot_fixture::build(&workspace_root()?, out.map(String::as_str))
        }
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
  screenshot-fixture [--out <dir>]    seed a synthetic profile for
                                      documentation screenshots. Nothing
                                      real is rendered, so nothing needs
                                      masking.

  verify-docs                         fail when README / AGENTS.md / the
                                      web docs drift from the code they
                                      describe (CLI verbs, Settings
                                      panes, data-dir databases, and that
                                      both copies of each screenshot
                                      match). Runs in CI.

  verify-screenshots                  report screenshots whose sources
                                      have moved since capture. NOT a CI
                                      gate: the comparison is per-
                                      directory, so adjacency reads as
                                      staleness, and re-capturing needs a
                                      macOS GUI that CI does not have.
                                      Run it before a release, or when
                                      you have changed a captured view.

  verify-cc-parity [--only <name>]    diff Rust merge output against
                                      parity-harness/fixtures/*/expected.json.
                                      Fails loudly on mismatch.

  cc-drift [--since <ver>]            has Claude Code moved under us?
           [--changelog <path>]       Reports version pins that have gone
                                      stale (each disables a surface
                                      silently) and every upstream
                                      release note that mentions a token
                                      from crates/xtask/cc-upstream-watch.md.
                                      NOT a CI gate: needs CC installed,
                                      and the version moves daily, so a
                                      gate would be permanently red.
                                      Defaults --since to the parity pin;
                                      fetches the changelog with `gh`
                                      unless --changelog is given.
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

    // The adapter is `parity-harness/dump.ts`, which drives an installed
    // Claude Code over its control protocol; it is not a source-tree
    // shim, so a CLAUDE_SRC checkout has nothing to do with it. Say so
    // instead of silently ignoring the variable.
    if std::env::var_os("CLAUDE_SRC").is_some() {
        eprintln!(
            "warning: CLAUDE_SRC is set, but nothing reads it — fixtures are \
             verified against an installed Claude Code with \
             `bun parity-harness/dump.ts --check`. See parity-harness/README.md §4."
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

    let input_path = fixture.join("input.json");
    let expected_path = fixture.join("expected.json");
    let input = read_json(&input_path)?;
    let expected = read_json(&expected_path)?;

    check_fixture_notes(fixture, pinned_cc_version, &input)?;

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
fn check_fixture_notes(
    fixture: &Path,
    pinned_cc_version: &str,
    input: &serde_json::Value,
) -> Result<()> {
    let notes_path = fixture.join("notes.md");
    let notes = std::fs::read_to_string(&notes_path).with_context(|| {
        format!(
            "read {} — every fixture must ship a notes.md recording how its \
             expected.json was established. See parity-harness/README.md §2.",
            notes_path.display()
        )
    })?;
    if let Some(problem) =
        notes_provenance_problem(&notes, pinned_cc_version, fixture_is_drivable(input))
    {
        bail!("{}: {problem}", notes_path.display());
    }
    Ok(())
}

/// `dump.ts` can install every layer except the policy ones.
fn fixture_is_drivable(input: &serde_json::Value) -> bool {
    input
        .get("policy")
        .and_then(|p| p.as_array())
        .is_none_or(|entries| {
            entries
                .iter()
                .all(|e| e.get("value").is_none_or(|v| v.is_null()))
        })
}

/// Every fixture carries exactly one kind of provenance.
///
/// A fixture `parity-harness/dump.ts` can drive must say it was verified by
/// running the pinned Claude Code — so moving the pin forces every such
/// fixture to be re-run, exactly as the single hand-derived pin used to.
/// Only a fixture dump.ts cannot drive may rest on a hand derivation; a
/// drivable one that claims it would be excusing itself from the check.
fn notes_provenance_problem(notes: &str, pinned: &str, drivable: bool) -> Option<String> {
    let verified = format!("Verified: claude-code@{pinned} by parity-harness/dump.ts");
    if notes.contains(&verified) {
        return None;
    }
    if let Some(at) = notes.find("Verified: claude-code@") {
        let rest = &notes[at + "Verified: claude-code@".len()..];
        let stale = rest.split_whitespace().next().unwrap_or("");
        return Some(format!(
            "was verified against {stale}, but the pin is {pinned} — run \
             `bun parity-harness/dump.ts --check` against {pinned} and update the line"
        ));
    }
    if notes.contains("Hand-derived: claude-code@") {
        return drivable.then(|| {
            format!(
                "claims a hand derivation, but dump.ts can drive this fixture — run \
                 `bun parity-harness/dump.ts --check` and record `{verified}`"
            )
        });
    }
    Some(format!(
        "records no provenance. Expected `{verified}` (after running \
         `bun parity-harness/dump.ts --check`), or `Hand-derived: claude-code@<version>` \
         for a fixture whose policy layers dump.ts cannot install"
    ))
}

/// Read `parity-harness/PINNED_CC_VERSION` — the Claude Code version the
/// machine-verified fixtures were last checked against by `dump.ts`.
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

#[cfg(test)]
mod parity_provenance_tests {
    use super::*;
    use serde_json::json;

    const PIN: &str = "2.1.274";

    fn policy(values: &[serde_json::Value]) -> serde_json::Value {
        json!({ "policy": values.iter().map(|v| json!({"origin": "remote", "value": v})).collect::<Vec<_>>() })
    }

    #[test]
    fn a_fixture_verified_against_the_pin_passes() {
        let notes = "Verified: claude-code@2.1.274 by parity-harness/dump.ts (2026-09-17).";
        assert_eq!(notes_provenance_problem(notes, PIN, true), None);
    }

    #[test]
    fn a_verification_against_an_older_pin_is_stale() {
        // Moving the pin must force every drivable fixture to be re-run.
        let notes = "Verified: claude-code@2.1.250 by parity-harness/dump.ts.";
        let problem = notes_provenance_problem(notes, PIN, true).unwrap();
        assert!(problem.contains("verified against 2.1.250"), "{problem}");
        assert!(problem.contains("the pin is 2.1.274"), "{problem}");
    }

    #[test]
    fn a_hand_derivation_is_accepted_only_where_dump_cannot_reach() {
        let notes = "Hand-derived: claude-code@2.1.88.";
        assert_eq!(notes_provenance_problem(notes, PIN, false), None);
        let problem = notes_provenance_problem(notes, PIN, true).unwrap();
        assert!(problem.contains("dump.ts can drive"), "{problem}");
    }

    #[test]
    fn notes_without_provenance_are_refused() {
        let problem = notes_provenance_problem("Derived by reading.", PIN, true).unwrap();
        assert!(problem.contains("records no provenance"), "{problem}");
    }

    #[test]
    fn only_a_populated_policy_layer_makes_a_fixture_undrivable() {
        assert!(fixture_is_drivable(&policy(&[json!(null), json!(null)])));
        assert!(fixture_is_drivable(&json!({})));
        assert!(!fixture_is_drivable(&policy(&[
            json!(null),
            json!({"a": 1})
        ])));
        // An empty object is still a layer someone has to install.
        assert!(!fixture_is_drivable(&policy(&[json!({})])));
    }
}
