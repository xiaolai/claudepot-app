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
use std::collections::{BTreeMap, BTreeSet};
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
/// `const NAME: &str = "value";` declarations in one file.
///
/// Needed because filenames are not always literals at the join site.
/// `board/mod.rs` writes
/// `claudepot_data_dir().join(BOARDS_DB_FILENAME)`, and a scan that
/// only reads string literals is structurally blind to it — which is
/// how `boards.db` stayed out of `KNOWN_DB_FILENAMES` long enough to
/// leak WAL sidecars, with a docs gate reporting green the whole time.
fn const_strings(text: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for line in text.lines() {
        let l = line.trim_start();
        let l = l.strip_prefix("pub ").unwrap_or(l);
        let l = l.strip_prefix("pub(crate) ").unwrap_or(l);
        let Some(rest) = l.strip_prefix("const ") else {
            continue;
        };
        let Some((name, tail)) = rest.split_once(':') else {
            continue;
        };
        let Some(v) = tail
            .split_once('"')
            .and_then(|(_, r)| r.find('"').map(|e| r[..e].to_string()))
        else {
            continue;
        };
        out.insert(name.trim().to_string(), v);
    }
    out
}

/// The filename passed to `.join(…)`, whether written as a literal or
/// as a const declared in the same file. `None` for anything else —
/// a variable, an expression, a `format!` — which the caller skips.
fn join_arg(after_join: &str, consts: &BTreeMap<String, String>) -> Option<String> {
    let rest = after_join.trim_start();
    if let Some(r) = rest.strip_prefix('"') {
        return r.find('"').map(|e| r[..e].to_string());
    }
    let ident: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if ident.is_empty() {
        return None;
    }
    consts.get(&ident).cloned()
}

fn production_only(text: &str) -> &str {
    let mut from = 0usize;
    while let Some(i) = text[from..].find("#[cfg(test)]") {
        let at = from + i;
        let after = text[at + "#[cfg(test)]".len()..].trim_start();
        if after.starts_with("mod ") || after.starts_with("pub mod ") {
            return &text[..at];
        }
        from = at + "#[cfg(test)]".len();
    }
    text
}

fn shipped_databases(repo: &Path) -> Result<BTreeSet<String>> {
    let mut out = BTreeSet::new();
    let root = repo.join("crates/claudepot-core/src");
    walk_rs(&root, &mut |text| {
        let consts = const_strings(text);
        let text = production_only(text);
        let mut from = 0usize;
        while let Some(i) = text[from..].find(".join(") {
            let at = from + i;
            from = at + 6;
            // Deliberately NOT anchored on `claudepot_data_dir()` the way
            // the JSON scan is: most databases are reached through their
            // own path helper rather than a direct join, so anchoring
            // would lose most of the coverage. `.db` is unambiguous
            // enough to match loosely — except for tempdir scratch files,
            // which always come through `dir.path().join(…)` and are the
            // one shape that must be excluded.
            if text[..at].ends_with(".path()") {
                continue;
            }
            if let Some(name) = join_arg(&text[from..], &consts) {
                if name.ends_with(".db") {
                    out.insert(name);
                }
            }
        }
    })?;
    Ok(out)
}

fn walk_rs(dir: &Path, f: &mut impl FnMut(&str)) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            walk_rs(&path, f)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            if let Ok(text) = std::fs::read_to_string(&path) {
                f(&text);
            }
        }
    }
    Ok(())
}

/// The `KNOWN_DB_FILENAMES` const, as declared in `db_housekeeping`.
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
    let shipped = shipped_databases(repo)?;

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

/// JSON state files under the Claudepot data dir, as the source
/// actually builds them.
///
/// Anchored on `claudepot_data_dir()` rather than on the `.json`
/// suffix, which is the whole difference from [`shipped_databases`].
/// `.db` is unambiguous — nothing else in this codebase ends in it —
/// but `.json` names CC's `settings.json`, `~/.claude.json`, bundle
/// entries like `manifest.json` and `tombstones.json`, and generated
/// specs. Matching the suffix alone would report a dozen files that do
/// not belong in AGENTS.md's data-dir list, and a check that cries wolf
/// is one people learn to skip.
///
/// **Known limit:** only the direct `claudepot_data_dir().join("x.json")`
/// chain is detected. A path assembled through an intermediate
/// binding (`let d = claudepot_data_dir(); d.join(…)`) is missed. No
/// such site exists today — the assertion below fails loudly if the
/// detector ever stops finding the files AGENTS.md documents, which is
/// what turns this from a check that *cannot* fail into one that has
/// been watched failing.
fn json_state_in(text: &str, consts: &BTreeMap<String, String>, out: &mut BTreeSet<String>) {
    const ANCHOR: &str = "claudepot_data_dir()";
    // Enough to clear a rustfmt line break plus indentation between the
    // anchor and its `.join(`, and short enough that an unrelated
    // `.join("x.json")` further down the function is not swept in.
    const WINDOW: usize = 160;

    let mut from = 0usize;
    while let Some(i) = text[from..].find(ANCHOR) {
        let start = from + i + ANCHOR.len();
        from = start;
        let window = &text[start..text.len().min(start + WINDOW)];
        let Some(j) = window.find(".join(") else {
            continue;
        };
        if let Some(name) = join_arg(&window[j + 6..], consts) {
            if name.ends_with(".json") {
                out.insert(name);
            }
        }
    }
}

fn shipped_json_state(repo: &Path) -> Result<BTreeSet<String>> {
    let mut out = BTreeSet::new();
    let root = repo.join("crates/claudepot-core/src");
    walk_rs(&root, &mut |text| {
        let consts = const_strings(text);
        json_state_in(production_only(text), &consts, &mut out)
    })?;
    Ok(out)
}

fn check_data_dir_json_state(repo: &Path, problems: &mut Vec<String>) -> Result<()> {
    let agents = read(repo, "AGENTS.md")?;
    let shipped = shipped_json_state(repo)?;

    // Guard against the detector silently finding nothing — a heuristic
    // that matches zero sites reports "all documented" forever. AGENTS.md
    // has named `agents.json` since the noun shipped, so its absence
    // means the anchor pattern moved, not that the file did.
    if !shipped.contains("agents.json") {
        problems.push(
            "the data-dir JSON detector found no `agents.json` — the \
             `claudepot_data_dir().join(…)` pattern it anchors on has changed, so \
             this check is now blind. Fix `shipped_json_state`, not AGENTS.md"
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

    /// The bug this replaced: cutting at the *first* `#[cfg(test)]`
    /// silently dropped every production item after an attributed
    /// method. `agent/store.rs` has one at line 791 and its test module
    /// at 1280, so `agents_file_path()` — which builds `agents.json` —
    /// was outside the scan entirely.
    #[test]
    fn production_only_cuts_at_the_test_module_not_an_attributed_item() {
        let src = "\
fn keep_me() {}
impl S {
    #[cfg(test)]
    fn helper() {}
}
fn also_keep_me() {}
#[cfg(test)]
mod tests {
    fn scratch() {}
}
";
        let kept = production_only(src);
        assert!(
            kept.contains("also_keep_me"),
            "production item after an attributed method must survive"
        );
        assert!(!kept.contains("scratch"), "the test module must be cut");
    }

    /// The blindness that let `boards.db` ship unlisted: its filename
    /// is a const, so `.join(BOARDS_DB_FILENAME)` carries no literal
    /// for a string scan to find.
    #[test]
    fn join_arg_resolves_a_const_filename() {
        let consts = const_strings("pub const BOARDS_DB_FILENAME: &str = \"boards.db\";\n");
        assert_eq!(
            consts.get("BOARDS_DB_FILENAME").map(String::as_str),
            Some("boards.db")
        );
        assert_eq!(
            join_arg("BOARDS_DB_FILENAME)", &consts).as_deref(),
            Some("boards.db")
        );
        // Literals still work, and an unresolvable expression is skipped
        // rather than guessed at.
        assert_eq!(join_arg("\"x.db\")", &consts).as_deref(), Some("x.db"));
        assert_eq!(join_arg("some_var)", &consts), None);
    }

    #[test]
    fn json_state_resolves_a_const_filename() {
        let consts = const_strings("const CACHE_FILENAME: &str = \"pricing-cache.json\";\n");
        let mut out = BTreeSet::new();
        json_state_in(
            "claudepot_data_dir().join(CACHE_FILENAME)",
            &consts,
            &mut out,
        );
        assert!(out.contains("pricing-cache.json"));
    }

    #[test]
    fn production_only_is_identity_without_a_test_module() {
        let src = "fn only() {}\n";
        assert_eq!(production_only(src), src);
    }

    #[test]
    fn json_state_matches_only_data_dir_joins() {
        let mut out = BTreeSet::new();
        json_state_in(
            r#"
            fn a() -> PathBuf { claudepot_data_dir().join("agents.json") }
            fn b() -> PathBuf { crate::paths::claudepot_data_dir().join("routes.json") }
            // Not the data dir: CC's own settings, and a bundle entry.
            fn c() -> PathBuf { config_dir.join("settings.json") }
            fn d() -> PathBuf { staged.join("manifest.json") }
            // Not JSON.
            fn e() -> PathBuf { claudepot_data_dir().join("corpus.db") }
            "#,
            &BTreeMap::new(),
            &mut out,
        );
        assert_eq!(
            out.into_iter().collect::<Vec<_>>(),
            vec!["agents.json".to_string(), "routes.json".to_string()]
        );
    }

    /// rustfmt breaks the chain across lines once the receiver is long
    /// enough; the scan has to survive that or it goes quietly blind.
    #[test]
    fn json_state_survives_a_wrapped_chain() {
        let mut out = BTreeSet::new();
        json_state_in(
            "crate::paths::claudepot_data_dir()\n            .join(\"usage_alert_state.json\")",
            &BTreeMap::new(),
            &mut out,
        );
        assert!(out.contains("usage_alert_state.json"));
    }
}
