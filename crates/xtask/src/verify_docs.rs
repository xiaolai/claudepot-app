//! `cargo xtask verify-docs` — fail when the docs disagree with the code.
//!
//! # Why this exists
//!
//! Documentation drift here is not sloppiness, it is *structural*: three
//! separate surfaces (`README.md`, `AGENTS.md`, the web product docs)
//! each restate facts that live in source, and nothing notices when the
//! source moves. A 2026-07-28 audit found six stale facts, only two of
//! which were introduced by the change that prompted the audit — the
//! rest had been wrong for releases.
//!
//! The failure mode that matters is `AGENTS.md`: agents read it *before
//! writing code*, so a wrong fact there does not mislead a reader, it
//! produces wrong code. "Six SQLite files" was wrong for exactly as long
//! as `corpus.db` existed.
//!
//! # What it checks
//!
//! Only facts that are **derivable from source**, so a failure is always
//! actionable and never a matter of taste:
//!
//! 1. every top-level CLI verb appears in README's command block;
//! 2. every Settings sub-pane id appears in README and the web settings
//!    page, and the spelled-out count matches;
//! 3. every `*.db` filename under the Claudepot data dir appears in
//!    AGENTS.md;
//! 4. every `*.json` state file joined onto `claudepot_data_dir()`
//!    appears in AGENTS.md — the JSON half of the same claim, which
//!    went unchecked until 2026-08-12 while AGENTS.md asserted the
//!    whole data-dir list was gated. `migrate-peers.json` was added
//!    and the gate stayed green;
//! 5. every documented screenshot exists and its two committed copies
//!    are byte-identical.
//!
//! Screenshot *freshness* is deliberately not here — see
//! [`verify_screenshots`], which is on demand.
//!
//! Prose quality is explicitly *not* checked. This catches "you added a
//! thing and forgot to say so", which is the whole observed failure.

use anyhow::{bail, Context, Result};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Screenshots, and the source whose UI they depict.
///
/// Kept here rather than in a separate manifest file because the check
/// is the only consumer; a second file would be one more thing to
/// forget. Add a row when a screenshot joins the docs.
const SCREENSHOTS: &[(&str, &[&str])] = &[
    (
        "accounts.png",
        &["src/sections/AccountsSection.tsx", "src/sections/accounts"],
    ),
    (
        "activities.png",
        &["src/sections/EventsSection.tsx", "src/sections/activities"],
    ),
    (
        "projects.png",
        &["src/sections/ProjectsSection.tsx", "src/sections/projects"],
    ),
    (
        "memory.png",
        &[
            "src/sections/SharedMemorySection.tsx",
            "src/sections/knowledge",
        ],
    ),
    (
        "keys.png",
        &["src/sections/KeysSection.tsx", "src/sections/keys"],
    ),
    (
        "global.png",
        &["src/sections/GlobalSection.tsx", "src/sections/global"],
    ),
    (
        "settings.png",
        &["src/sections/SettingsSection.tsx", "src/sections/settings"],
    ),
    (
        "third-parties.png",
        &[
            "src/sections/ThirdPartySection.tsx",
            "src/sections/third-party",
        ],
    ),
    (
        "automations.png",
        &["src/sections/AgentsSection.tsx", "src/sections/agents"],
    ),
];

pub fn verify_docs(repo: &Path) -> Result<()> {
    let mut problems: Vec<String> = Vec::new();

    check_cli_verbs(repo, &mut problems)?;
    check_settings_panes(repo, &mut problems)?;
    check_data_dir_databases(repo, &mut problems)?;
    check_data_dir_json_state(repo, &mut problems)?;
    // Screenshot *freshness* is `cargo xtask verify-screenshots`, on
    // demand — see that function for why it is not a pull-request gate.
    check_screenshot_pairs(repo, &mut problems)?;
    check_cc_env_spec(repo, &mut problems)?;

    if problems.is_empty() {
        println!(
            "verify-docs: ok — CLI verbs, Settings panes, databases, data-dir JSON state and the \
             cc-env spec all in sync"
        );
        return Ok(());
    }
    let mut msg = format!("verify-docs found {} drift(s):", problems.len());
    for p in &problems {
        msg.push_str("\n  - ");
        msg.push_str(p);
    }
    bail!(msg)
}

fn read(repo: &Path, rel: &str) -> Result<String> {
    let p: PathBuf = repo.join(rel);
    std::fs::read_to_string(&p).with_context(|| format!("read {}", p.display()))
}

// ─── 1. CLI verbs ────────────────────────────────────────────────────

/// Top-level variants of `enum Commands` in the CLI's `main.rs`.
///
/// Parsed rather than hand-listed: a hand-listed set is one more place
/// to forget, which is the bug being fixed.
fn shipped_cli_verbs(src: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let Some(start) = src.find("enum Commands {") else {
        return out;
    };
    let body = &src[start..];
    let mut depth = 0usize;
    for line in body.lines() {
        let trimmed = line.trim_start();
        // Variants sit at exactly one level of nesting inside the enum.
        if depth == 1 && !trimmed.starts_with("//") && !trimmed.starts_with("#[") {
            if let Some(name) = trimmed
                .split(|c: char| !c.is_alphanumeric())
                .next()
                .filter(|s| s.chars().next().is_some_and(char::is_uppercase))
            {
                // A variant line ends in `{`, `,` or `(`.
                if trimmed.ends_with('{') || trimmed.ends_with(',') || trimmed.contains('(') {
                    out.insert(name.to_lowercase());
                }
            }
        }
        depth += line.matches('{').count();
        depth = depth.saturating_sub(line.matches('}').count());
        if depth == 0 && line.contains('}') && !out.is_empty() {
            break;
        }
    }
    out
}

fn check_cli_verbs(repo: &Path, problems: &mut Vec<String>) -> Result<()> {
    let src = read(repo, "crates/claudepot-cli/src/main.rs")?;
    let readme = read(repo, "README.md")?;
    // Verbs documented in README's command block. Two shapes occur:
    // `claudepot session   list | move` (one verb per line) and
    // `claudepot export / import` (two verbs sharing a line). Collecting
    // every word from lines that begin `claudepot ` handles both without
    // a special case, and without the `import` false positive a plain
    // `contains("claudepot import")` produces.
    let documented: BTreeSet<String> = readme
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("claudepot "))
        .flat_map(|l| {
            l.split(|c: char| !c.is_ascii_alphanumeric() && c != '-')
                .map(str::to_lowercase)
                .collect::<Vec<_>>()
        })
        .collect();

    for verb in shipped_cli_verbs(&src) {
        if !documented.contains(&verb) {
            problems.push(format!(
                "CLI verb `{verb}` ships but is absent from README's command block"
            ));
        }
    }
    Ok(())
}

// ─── 2. Settings panes ───────────────────────────────────────────────

fn shipped_settings_panes(src: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for line in src.lines() {
        let Some(rest) = line.trim_start().strip_prefix("{ id: \"") else {
            continue;
        };
        if let Some(end) = rest.find('"') {
            out.insert(rest[..end].to_string());
        }
    }
    out
}

/// English words for the pane counts we could plausibly reach. Docs
/// spell the number out, so a bare integer comparison would not catch
/// "Thirteen" against fourteen panes.
fn spelled(n: usize) -> Option<&'static str> {
    Some(match n {
        10 => "Ten",
        11 => "Eleven",
        12 => "Twelve",
        13 => "Thirteen",
        14 => "Fourteen",
        15 => "Fifteen",
        16 => "Sixteen",
        17 => "Seventeen",
        18 => "Eighteen",
        19 => "Nineteen",
        20 => "Twenty",
        _ => return None,
    })
}

/// The pane table moved out of `SettingsSection.tsx` into its own
/// JSX-free module so the ⌘K palette could import it without pulling
/// the lazy Settings chunk into the main bundle. The list is still the
/// single source of truth — only its address changed.
const SETTINGS_PANES_SRC: &str = "src/sections/settings/panes.ts";

fn check_settings_panes(repo: &Path, problems: &mut Vec<String>) -> Result<()> {
    let src = read(repo, SETTINGS_PANES_SRC)?;
    let panes = shipped_settings_panes(&src);
    if panes.is_empty() {
        bail!(
            "could not parse any Settings panes from {SETTINGS_PANES_SRC} — \
             the SETTINGS_PANES shape changed, fix this check"
        );
    }

    let web_rel = "web/src/app/(reader)/app/features/settings/page.mdx";
    let surfaces = [
        ("README.md", read(repo, "README.md")?),
        (web_rel, read(repo, web_rel)?),
    ];

    for (name, text) in &surfaces {
        let lower = text.to_lowercase();
        for pane in &panes {
            if !lower.contains(pane.as_str()) {
                problems.push(format!(
                    "Settings pane `{pane}` ships but is not mentioned in {name}"
                ));
            }
        }
        if let Some(word) = spelled(panes.len()) {
            // Only complain when the doc states *some* count and it is
            // the wrong one; a page that never counts is fine.
            let states_a_count = (10..=20)
                .filter_map(spelled)
                .any(|w| text.contains(&format!("{w} sub-pane")));
            if states_a_count && !text.contains(&format!("{word} sub-pane")) {
                problems.push(format!(
                    "{name} states the wrong Settings pane count — there are {} ({word})",
                    panes.len()
                ));
            }
        }
    }
    Ok(())
}

// ─── 3. Databases under the data dir ─────────────────────────────────

/// `*.db` filenames joined onto `claudepot_data_dir()` anywhere in core.
/// Everything before the file's `#[cfg(test)] mod …` block.
///
/// Test modules are full of scratch names (`test.db`, `fresh.db`,
/// `a.db`) that nobody should document, and a check that reports those
/// trains people to ignore it.
///
/// Cutting at the first `#[cfg(test)]` — what this did originally — is
/// wrong, and was silently losing coverage. `#[cfg(test)]` also marks
/// individual methods and fields: `agent/store.rs` carries one at line
/// 791 while its test module starts at 1280, so ~490 lines of
/// production code (including `agents_file_path()`, which builds
/// `agents.json`) sat outside the scan. The cut must therefore be the
/// attribute that introduces a *module*, not any occurrence of it.
fn known_db_filenames(src: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let Some(start) = src.find("KNOWN_DB_FILENAMES") else {
        return out;
    };
    let body = &src[start..];
    let Some(end) = body.find("];") else {
        return out;
    };
    let mut rest = &body[..end];
    while let Some(i) = rest.find('"') {
        rest = &rest[i + 1..];
        if let Some(j) = rest.find('"') {
            let name = &rest[..j];
            if name.ends_with(".db") {
                out.insert(name.to_string());
            }
            rest = &rest[j + 1..];
        }
    }
    out
}

fn check_data_dir_databases(repo: &Path, problems: &mut Vec<String>) -> Result<()> {
    let agents = read(repo, "AGENTS.md")?;
    let shipped = crate::data_dir_scan::scan(&repo.join("crates/claudepot-core/src"))?.dbs;

    // The WAL-cleanup list must cover every database that actually
    // ships. Its own doc comment calls it exhaustive, and it was not:
    // `boards.db` and `corpus.db` both shipped without being added, so
    // both leaked `*.db-wal` sidecars until 2026-08-12. Nothing catches
    // that at runtime — the leak is small and silent, which is exactly
    // the kind of omission a gate has to notice instead of a person.
    let housekeeping = read(repo, "crates/claudepot-core/src/db_housekeeping.rs")?;
    let known = known_db_filenames(&housekeeping);
    if known.is_empty() {
        problems.push(
            "could not parse `KNOWN_DB_FILENAMES` from db_housekeeping.rs — the WAL-cleanup \
             cross-check is blind. Fix `known_db_filenames`, not the const"
                .to_string(),
        );
    }

    for db in &shipped {
        if !agents.contains(db.as_str()) {
            problems.push(format!(
                "`{db}` lives under the Claudepot data dir but AGENTS.md never names it \
                 — an agent reading AGENTS.md will not know it exists"
            ));
        }
        if !known.is_empty() && !known.contains(db) {
            problems.push(format!(
                "`{db}` ships but is absent from `KNOWN_DB_FILENAMES` in db_housekeeping.rs \
                 — its `*.db-wal` sidecar will never be cleaned up"
            ));
        }
    }
    Ok(())
}

fn check_data_dir_json_state(repo: &Path, problems: &mut Vec<String>) -> Result<()> {
    let agents = read(repo, "AGENTS.md")?;
    let shipped = crate::data_dir_scan::scan(&repo.join("crates/claudepot-core/src"))?.jsons;

    // Guard against the detector silently finding nothing — a heuristic
    // that matches zero sites reports "all documented" forever. AGENTS.md
    // has named `agents.json` since the noun shipped, so its absence
    // means the anchor pattern moved, not that the file did.
    if !shipped.contains("agents.json") {
        problems.push(
            "the data-dir JSON detector found no `agents.json` — the \
             `claudepot_data_dir().join(…)` pattern it anchors on has changed, so \
             this check is now blind. Fix `data_dir_scan`, not AGENTS.md"
                .to_string(),
        );
        return Ok(());
    }

    for name in shipped {
        if !agents.contains(&name) {
            problems.push(format!(
                "`{name}` lives under the Claudepot data dir but AGENTS.md never names it \
                 — an agent reading AGENTS.md will not know it exists"
            ));
        }
    }
    Ok(())
}

// ─── 4. Screenshot freshness ─────────────────────────────────────────

/// Last commit date touching `paths`, as `YYYY-MM-DD`.
///
/// Commit dates, not mtimes: `git checkout` rewrites mtimes, so a fresh
/// clone reports every screenshot as newer than its source and the check
/// silently passes. That is exactly how eight screenshots sat three
/// months stale without anyone noticing.
fn last_commit_date(repo: &Path, paths: &[&str]) -> Option<String> {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["log", "-1", "--format=%ad", "--date=short", "--"])
        .args(paths)
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// Screenshot facts that are cheap, deterministic and fixable *here*:
/// the file exists, and the two committed copies are the same bytes.
///
/// A drifted pair means the README and the web docs show different apps,
/// and the fix is a file copy — something a CI failure can actually tell
/// you to do. Contrast [`verify_screenshots`], which cannot say that.
fn check_screenshot_pairs(repo: &Path, problems: &mut Vec<String>) -> Result<()> {
    for (shot, _sources) in SCREENSHOTS {
        let asset = format!("assets/screenshots/{shot}");
        let web = format!("web/public/screenshots/{shot}");
        if !repo.join(&asset).exists() {
            problems.push(format!("{asset} is referenced by the docs but missing"));
            continue;
        }
        // The two copies are duplicated by hand today; a drifted pair
        // means one surface shows a different app than the other.
        if repo.join(&web).exists() {
            let a = std::fs::read(repo.join(&asset)).ok();
            let b = std::fs::read(repo.join(&web)).ok();
            if a.is_some() && a != b {
                problems.push(format!(
                    "{shot} differs between assets/screenshots and web/public/screenshots"
                ));
            }
        }
    }
    Ok(())
}

/// `cargo xtask verify-screenshots` — report screenshots whose UI has
/// been touched since they were captured.
///
/// # Why this is on demand rather than part of `verify-docs`
///
/// It was a CI gate, and it was the wrong shape for one on two counts.
///
/// **It cannot tell staleness from adjacency.** The comparison is
/// per-*directory* commit dates, so any edit under `src/sections/projects`
/// — a new sibling component, a renamed prop, a test file — reads as
/// "the UI changed", including when the captured view provably did not
/// move. `projects.png` shows the Projects list; a change to the
/// move-session dialog flags it anyway.
///
/// **Its remediation cannot run where it fires.** Re-capturing needs a
/// macOS GUI session, a Vite dev server, a debug build carrying the MCP
/// bridge, and a windowed app driven over a WebSocket. CI has none of
/// those, so a red run there is a wall, not a signal — and a gate whose
/// fix is unrunnable at the point of failure is the same dynamic that
/// turned `--no-verify` into a reflex for the release validators.
///
/// It stays a real check — loud, exit-code-bearing — so that running it
/// still means something. The original failure it caught (eight
/// screenshots three months stale) is a periodic-sweep problem, not a
/// per-pull-request one.
pub fn verify_screenshots(repo: &Path) -> Result<()> {
    if !repo.join(".git").exists() {
        bail!("verify-screenshots needs a git checkout — it compares commit dates");
    }
    let mut stale: Vec<String> = Vec::new();
    for (shot, sources) in SCREENSHOTS {
        let asset = format!("assets/screenshots/{shot}");
        if !repo.join(&asset).exists() {
            stale.push(format!("{asset} is referenced by the docs but missing"));
            continue;
        }
        let (Some(shot_at), Some(src_at)) = (
            last_commit_date(repo, &[&asset]),
            last_commit_date(repo, sources),
        ) else {
            continue;
        };
        if src_at > shot_at {
            stale.push(format!(
                "{shot}: captured {shot_at}, its sources last moved {src_at}"
            ));
        }
    }
    if stale.is_empty() {
        println!(
            "verify-screenshots: ok — all {} shots are at least as new as their sources",
            SCREENSHOTS.len()
        );
        return Ok(());
    }
    let mut msg = format!("verify-screenshots: {} shot(s) may be stale:", stale.len());
    for s in &stale {
        msg.push_str("\n  - ");
        msg.push_str(s);
    }
    msg.push_str(
        "\n\nRe-capture (macOS, GUI session required):\n\
         \x20 cargo xtask screenshot-fixture\n\
         \x20 pnpm dev &\n\
         \x20 cargo build -p claudepot-tauri\n\
         \x20 HOME=/tmp/claudepot-demo-home ./target/debug/claudepot-tauri &\n\
         \x20 pnpm screenshots\n\n\
         Adjacency is not staleness: if the captured view genuinely did not \
         change, there is nothing to re-capture.",
    );
    bail!(msg)
}

// ─── 5. The embedded Claude Code env-var spec ────────────────────────

/// Re-run `scripts/build-cc-env-spec.py --check`.
///
/// The script regenerates `crates/claudepot-core/data/cc-env-spec.json` from
/// the committed evidence and compares byte-for-byte, then runs the
/// hand-authored golden vectors. Both halves are offline and hermetic: a
/// committed script reading gitignored inputs would not be reproducible, and
/// a checksum sitting next to its own artifact would only prove the two were
/// edited together.
///
/// What this deliberately does **not** check is the *user's* Claude Code
/// version. That is a runtime concern — the snapshot is valid only on an
/// exact match, decided by `cc_env::spec::CrosscheckValidity` when the pane
/// renders — not a build one.
fn check_cc_env_spec(repo: &Path, problems: &mut Vec<String>) -> Result<()> {
    let script = repo.join("scripts/build-cc-env-spec.py");
    if !script.exists() {
        problems.push(format!(
            "{} is missing — the embedded cc-env spec would have no committed producer",
            script.display()
        ));
        return Ok(());
    }
    let out = match std::process::Command::new("python3")
        .arg(&script)
        .arg("--check")
        .current_dir(repo)
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            // No python3 is an environment gap, not doc drift. Say so rather
            // than reporting a green check we did not run.
            problems.push(format!(
                "could not run {} --check: {e} (install python3 to gate the cc-env spec)",
                script.display()
            ));
            return Ok(());
        }
    };
    if !out.status.success() {
        let detail = String::from_utf8_lossy(&out.stderr)
            .lines()
            .chain(String::from_utf8_lossy(&out.stdout).lines())
            .filter(|l| !l.trim().is_empty())
            .collect::<Vec<_>>()
            .join(" / ");
        problems.push(format!("cc-env spec check failed: {detail}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_top_level_command_variants_only() {
        let src = r#"
#[derive(Subcommand)]
enum Commands {
    /// doc
    Account {
        #[command(subcommand)]
        action: AccountAction,
    },
    Corpus {
        #[command(subcommand)]
        action: CorpusAction,
    },
    Status,
}

#[derive(Subcommand)]
enum AccountAction {
    List,
    Add,
}
"#;
        let v = shipped_cli_verbs(src);
        assert!(v.contains("account") && v.contains("corpus") && v.contains("status"));
        assert!(
            !v.contains("list") && !v.contains("add"),
            "subcommand variants must not be counted as top-level verbs: {v:?}"
        );
    }

    #[test]
    fn parses_settings_pane_ids() {
        let src = r#"
  { id: "general",   label: "General",   glyph: NF.sliders, group: "core" },
  { id: "retention", label: "Retention", glyph: NF.archive, group: "core" },
"#;
        let p = shipped_settings_panes(src);
        assert!(p.contains("general") && p.contains("retention"));
        assert_eq!(p.len(), 2);
    }

    #[test]
    fn spells_the_counts_the_docs_actually_use() {
        assert_eq!(spelled(13), Some("Thirteen"));
        assert_eq!(spelled(14), Some("Fourteen"));
        assert_eq!(spelled(99), None);
    }
}
