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
//!    are byte-identical;
//! 6. every `Connection::open` in production code reaches
//!    `db_pragmas::apply_standard_pragmas`. Not a docs fact, but the
//!    same shape of failure and it belongs beside check 3: that one
//!    gates WAL *cleanup* coverage, this one gates WAL *growth* bounds,
//!    and `corpus.db` was missing from both for as long as it existed;
//! 7. the website's hand-copied icon assets still track
//!    `assets/icon-set/`. `scripts/regen-icons.sh` covers the app
//!    ladder only, so the web copies are maintained by memory — and in
//!    v0.4.12 memory missed one: the favicon was redrawn and the nav
//!    logo was left pointing at `pixel-*` masters that the same commit
//!    deleted. claudepot.com served two different marks for two
//!    releases.
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
    check_event_channels_have_listeners(repo, &mut problems)?;
    // Screenshot *freshness* is `cargo xtask verify-screenshots`, on
    // demand — see that function for why it is not a pull-request gate.
    check_screenshot_pairs(repo, &mut problems)?;
    check_shortcut_gate_is_shared(repo, &mut problems)?;
    check_contrast_overrides_win(repo, &mut problems)?;
    check_shortcut_table_has_handlers(repo, &mut problems)?;
    check_no_undefined_tokens(repo, &mut problems)?;
    check_web_icon_provenance(repo, &mut problems)?;
    check_cc_env_spec(repo, &mut problems)?;

    if problems.is_empty() {
        println!(
            "verify-docs: ok — CLI verbs, Settings panes, databases, data-dir JSON state, the \
             shortcut gate, web icon assets and the cc-env spec all in sync"
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
    let shipped = crate::data_dir_scan::scan(repo)?.dbs;

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

    // Layer 1 of the same WAL defense: the housekeeping list bounds
    // *cleanup at startup*, `apply_standard_pragmas` bounds *growth*.
    // A store that skips the helper is covered only by the startup
    // pass, which backs off after 1s whenever another process holds
    // the file — exactly the situation a long index run creates.
    // `corpus.rs` hand-rolled its pragmas and silently dropped
    // `journal_size_limit` / `wal_autocheckpoint` for as long as it
    // existed, on the largest database in the app.
    let scan = crate::data_dir_scan::scan(repo)?;
    for open in &scan.unpragmad_opens {
        problems.push(format!(
            "`{}::{}` calls `Connection::open` without `db_pragmas::apply_standard_pragmas` \
             — its WAL has no size bound. Call the helper rather than hand-rolling a pragma \
             batch; a hand-rolled batch that is right today is a divergence tomorrow",
            open.file, open.function
        ));
    }
    Ok(())
}

// ─── 7. Event channels have a subscriber ─────────────────────────────

/// Channels the renderer deliberately does not subscribe to, and why.
///
/// An exception has to be written down somewhere a reader will find it
/// *while looking at the failure*, or the next person deletes a real
/// subscriber and assumes the red gate is the usual noise. Both
/// directions are validated in [`check_event_channels_have_listeners`],
/// so an entry cannot outlive the reason for it.
const UNSUBSCRIBED_BY_DESIGN: &[(&str, &str)] = &[(
    "agent-event-dispatched",
    "a successful event-agent run already lands in RunHistoryPanel with its structured \
     output; a toast per fire would spam every settled-session narration. \
     `useAgentEventToasts.ts` documents it and its test asserts the channel stays unsubscribed",
)];

/// Channel names declared as `pub const … : &str = "…"` in `events.rs`,
/// plus any emitted as a bare literal elsewhere in the Tauri crate.
///
/// Both halves matter: the tray and app-menu files historically kept
/// their channel strings at the emit site rather than in `events.rs`,
/// and three of the four channels that turned out to have no listener
/// were exactly those.
fn declared_channels(repo: &Path) -> Result<BTreeSet<String>> {
    let mut out = BTreeSet::new();

    let events = read(repo, "src-tauri/src/events.rs")?;
    for line in events.lines() {
        let t = line.trim();
        if !t.starts_with("pub const ") || !t.contains("&str") {
            continue;
        }
        if let Some(v) = t.split('"').nth(1) {
            out.insert(v.to_string());
        }
    }

    let mut files = Vec::new();
    collect_rs(&repo.join("src-tauri/src"), &mut files)?;
    for f in files {
        let src = std::fs::read_to_string(&f)?;
        let mut rest = src.as_str();
        while let Some(i) = rest.find("emit(\"") {
            rest = &rest[i + "emit(\"".len()..];
            if let Some(j) = rest.find('"') {
                out.insert(rest[..j].to_string());
                rest = &rest[j + 1..];
            }
        }
    }
    Ok(out)
}

fn collect_rs(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(dir)? {
        let p = entry?.path();
        if p.is_dir() {
            collect_rs(&p, out)?;
        } else if p.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(p);
        }
    }
    Ok(())
}

/// Strip `//` and `/* */` comments, without being fooled by the same
/// characters inside a string literal.
///
/// Needed because the thing being searched for is a channel *name*,
/// and prose mentions one constantly — `useTrayBridge`'s own header
/// comment lists all four Desktop channels. Searching raw text made
/// the gate pass with every real subscriber deleted, which is the
/// failure it exists to catch.
fn strip_ts_comments(src: &str) -> String {
    let b = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    // `Some(q)` while inside a string opened with quote byte `q`.
    let mut quote: Option<u8> = None;

    while i < b.len() {
        let c = b[i];
        if let Some(q) = quote {
            // Skip an escaped character wholesale so `"\\\""` does not
            // read as the end of the string.
            if c == b'\\' && i + 1 < b.len() {
                out.push(c as char);
                out.push(b[i + 1] as char);
                i += 2;
                continue;
            }
            if c == q {
                quote = None;
            }
            out.push(c as char);
            i += 1;
            continue;
        }
        if c == b'"' || c == b'\'' || c == b'`' {
            quote = Some(c);
            out.push(c as char);
            i += 1;
            continue;
        }
        if c == b'/' && i + 1 < b.len() {
            if b[i + 1] == b'/' {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            if b[i + 1] == b'*' {
                i += 2;
                while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                    i += 1;
                }
                i = (i + 2).min(b.len());
                continue;
            }
        }
        // Non-ASCII bytes are copied as-is; only ASCII delimiters
        // matter to this scanner, and the search is for ASCII channel
        // names.
        out.push(c as char);
        i += 1;
    }
    out
}

/// Comment-stripped concatenation of every **shipped** `.ts`/`.tsx`
/// file under `src/`.
///
/// Two exclusions, both established by deleting the real subscribers
/// and checking the gate actually goes red:
///
/// - **Test files.** A test that fires a channel names it, so
///   `useTrayBridge.test.tsx` alone satisfied "somebody references
///   `tray-desktop-switched`" while the shipped hook subscribed to
///   nothing.
/// - **Comments.** With tests excluded the gate *still* passed,
///   because the hook's own header comment lists the channels it
///   handles. A doc mention is not a subscription.
fn frontend_sources(repo: &Path) -> Result<String> {
    fn is_test_file(p: &Path) -> bool {
        p.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.contains(".test.") || n.contains(".spec."))
    }

    fn walk(dir: &Path, buf: &mut String) -> Result<()> {
        // `src/test/` is fixtures and harness setup, not shipped code.
        if dir.file_name().and_then(|n| n.to_str()) == Some("test") {
            return Ok(());
        }
        for entry in std::fs::read_dir(dir)? {
            let p = entry?.path();
            if p.is_dir() {
                walk(&p, buf)?;
            } else if matches!(
                p.extension().and_then(|e| e.to_str()),
                Some("ts") | Some("tsx")
            ) && !is_test_file(&p)
            {
                buf.push_str(&strip_ts_comments(&std::fs::read_to_string(&p)?));
                buf.push('\n');
            }
        }
        Ok(())
    }
    let mut buf = String::new();
    walk(&repo.join("src"), &mut buf)?;
    Ok(buf)
}

/// Every backend-emitted channel must be named somewhere in `src/`.
///
/// # Why this is a gate and not a unit test
///
/// `events.rs` carried a test called "wire-contract lock" whose body
/// was `assert_eq!(DESKTOP_ADOPTED, "desktop-adopted")` twenty-one
/// times over — each constant compared to its own literal. It could
/// only ever catch a rename, while its docstring claimed to protect
/// the contract with the renderer.
///
/// It did not. Seven declared channels had zero subscribers in `src/`:
/// `tray-desktop-switched`, `tray-desktop-switch-failed`,
/// `tray-desktop-launch-failed` and `desktop-reconciled` (so a tray
/// Desktop swap left the UI stale and a FAILED one produced no signal
/// anywhere), plus `desktop-adopted`, `desktop-cleared` and
/// `desktop-running-changed` (dead weight — the invoking command
/// already returned the same facts). A tautology inside one crate
/// cannot see the other end of a cross-boundary contract; only a check
/// that reads both sides can.
///
/// Deliberately a *name* search rather than a listener-registration
/// parse: the renderer reaches channels through `useTauriEvent`,
/// `useTauriEvents` object keys, and shared constants in
/// `lib/events.ts`. Matching the string covers all three, and the
/// failure being prevented is "nobody mentions this at all".
fn check_event_channels_have_listeners(repo: &Path, problems: &mut Vec<String>) -> Result<()> {
    let declared = declared_channels(repo)?;
    if declared.len() < 10 {
        problems.push(format!(
            "only parsed {} event channels from src-tauri — the listener cross-check is \
             effectively blind. Fix `declared_channels`, not the events",
            declared.len()
        ));
        return Ok(());
    }
    let frontend = frontend_sources(repo)?;

    // The allowlist is kept honest in both directions below: an entry
    // naming a channel that no longer exists, or one that has since
    // gained a subscriber, is itself a drift. Otherwise silencing this
    // gate would be a one-line edit with no cost, which is how a gate
    // stops meaning anything.
    for (ch, _) in UNSUBSCRIBED_BY_DESIGN {
        if !declared.contains(*ch) {
            problems.push(format!(
                "`{ch}` is listed in UNSUBSCRIBED_BY_DESIGN but is no longer emitted \
                 — delete the stale entry"
            ));
        }
    }

    for ch in &declared {
        if let Some((_, why)) = UNSUBSCRIBED_BY_DESIGN.iter().find(|(c, _)| c == ch) {
            if frontend.contains(&format!("\"{ch}\"")) {
                problems.push(format!(
                    "`{ch}` is listed in UNSUBSCRIBED_BY_DESIGN ({why}) but the renderer now \
                     subscribes to it — remove the entry or the subscription"
                ));
            }
            continue;
        }
        // Quoted forms only. Every real subscription spells the
        // channel as a `"…"` / `'…'` literal — `useTauriEvent("x", …)`,
        // a `useTauriEvents({ "x": … })` key, or a shared constant in
        // `lib/events.ts`. Requiring the quotes is a second, cheaper
        // guard against a bare mention counting as a subscriber.
        let quoted = [format!("\"{ch}\""), format!("'{ch}'")];
        if !quoted.iter().any(|q| frontend.contains(q.as_str())) {
            problems.push(format!(
                "channel `{ch}` is emitted by the backend but named nowhere under `src/` \
                 — either the renderer is missing a subscriber (a tray action that \
                 silently does nothing) or the channel is dead and should be deleted"
            ));
        }
    }
    Ok(())
}

fn check_data_dir_json_state(repo: &Path, problems: &mut Vec<String>) -> Result<()> {
    let agents = read(repo, "AGENTS.md")?;
    let shipped = crate::data_dir_scan::scan(repo)?.jsons;

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

/// Custom-property names DECLARED in `src` — i.e. `--x:` anywhere, not
/// only at the start of a line.
///
/// The line-initial form worked only because `tokens.css` happens to
/// put one declaration per line. A guard that depends on the
/// formatting of the file it inspects is one reflow away from
/// silently checking nothing, and the fixture tests caught exactly
/// that: `:root { --fg: red; }` on one line parsed as zero
/// declarations, so every reference looked undefined.
///
/// Requiring the `:` is what separates a declaration from a
/// reference — `var(--fg)` and `var(--fg, red)` are followed by `)`
/// or `,`, never `:`.
fn declared_properties(src: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let b = src.as_bytes();
    let mut i = 0usize;
    while let Some(rel) = src[i..].find("--") {
        let at = i + rel;
        let name_start = at + 2;
        let mut j = name_start;
        while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'-') {
            j += 1;
        }
        if j > name_start {
            let mut k = j;
            while k < b.len() && (b[k] as char).is_whitespace() {
                k += 1;
            }
            if k < b.len() && b[k] == b':' {
                out.insert(format!("--{}", &src[name_start..j]));
            }
        }
        i = j.max(at + 2);
    }
    out
}

/// Every `var(--token)` in the app must name a token `tokens.css`
/// actually declares.
///
/// # Why this is not covered by the no-raw-values rule
///
/// `design.md` requires every colour, size and spacing to come from
/// `tokens.css`, and the stylesheets obey it — no hex, rgb or hsl
/// literal appears anywhere. But that rule only catches values that
/// *look* wrong. A MISSPELLED token passes it perfectly: `var(--fg-2)`
/// is not a raw value, reviews read it as a token, and CSS resolves an
/// undefined custom property to nothing — so the declaration is
/// dropped and the element silently inherits instead.
///
/// The 2026-08 token migration found 43 such references across 16
/// invented names: `--fg-2` / `--fg-3` where the scale is
/// `--fg-muted` / `--fg-faint`, `--rad-sm` / `--radius-sm` /
/// `--rad-2` where it is `--r-0…--r-5`, `--fs-12` and `--fs-14`
/// where it is `--fs-xs`. Every one rendered as an unset property and
/// nothing anywhere reported it.
fn check_no_undefined_tokens(repo: &Path, problems: &mut Vec<String>) -> Result<()> {
    let tokens_src = strip_comments(&read(repo, "src/styles/tokens.css")?);
    let declared = declared_properties(&tokens_src);
    if declared.is_empty() {
        problems.push("tokens.css declares no custom properties — the check cannot run".into());
        return Ok(());
    }

    let mut files: Vec<PathBuf> = Vec::new();
    collect_ts_paths(&repo.join("src"), &mut files);
    collect_css_paths(&repo.join("src"), &mut files);

    // Custom properties can also be declared LOCALLY — on an element
    // via a JSX style object (`["--rank-col"]: "var(--sp-28)"`) or in
    // a component stylesheet rule. Those are valid and deliberately
    // scoped; the first draft of this check knew only about
    // tokens.css and reported `--rank-col` as undefined when it is set
    // on the very element that reads it. Collect them first.
    let mut local: BTreeSet<String> = BTreeSet::new();
    {
        let mut all: Vec<PathBuf> = Vec::new();
        collect_ts_paths(&repo.join("src"), &mut all);
        collect_css_paths(&repo.join("src"), &mut all);
        for path in &all {
            let Ok(src) = std::fs::read_to_string(path) else {
                continue;
            };
            // `"--x":` / `'--x':` (JSX style key) and `--x:` (CSS decl).
            for pat in ["\"--", "'--"] {
                let mut from = 0usize;
                while let Some(rel) = src[from..].find(pat) {
                    let at = from + rel + pat.len();
                    let end = src[at..]
                        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
                        .map(|i| at + i)
                        .unwrap_or(src.len());
                    // The QUOTE is the pattern's first char, not its last —
                    // `pat` is `"--`, so `next_back()` yields `-` and the
                    // match could never succeed.
                    if src[end..].starts_with(pat.chars().next().unwrap()) {
                        local.insert(format!("--{}", &src[at..end]));
                    }
                    from = end.max(at + 1);
                }
            }
        }
    }

    let mut missing: BTreeSet<String> = BTreeSet::new();
    let mut scanned = 0usize;
    for path in files {
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        // Comments stripped. A comment EXPLAINING a token — including
        // one explaining that the token does not exist — otherwise
        // reads as a reference to it. This is the third guard in this
        // change to have been fooled by its own documentation; the
        // pattern is now assumed rather than discovered.
        let src = strip_comments(&raw);
        let mut from = 0usize;
        while let Some(rel) = src[from..].find("var(--") {
            let at = from + rel + 4;
            let end = src[at..]
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-'))
                .map(|i| at + i)
                .unwrap_or(src.len());
            let name = &src[at..end];
            scanned += 1;
            // A `var(--x, fallback)` is legitimate even when --x is
            // absent; only the bare form is a silent drop.
            let bare = src[end..].starts_with(')');
            if bare && !declared.contains(name) && !local.contains(name) {
                missing.insert(format!(
                    "{} (first seen in {})",
                    name,
                    path.strip_prefix(repo).unwrap_or(&path).display()
                ));
            }
            from = end;
        }
    }
    if scanned == 0 {
        problems.push(
            "no var(--token) references found under src/ — the scan is broken, not the \
             codebase"
                .into(),
        );
    }
    for m in missing {
        problems.push(format!(
            "{m} is referenced but never declared in tokens.css — CSS drops the whole \
             declaration, so the element silently inherits instead"
        ));
    }
    Ok(())
}

/// `.css` paths under `dir`, appended to `out`.
fn collect_css_paths(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.filter_map(Result::ok) {
        let path = e.path();
        if path.is_dir() {
            collect_css_paths(&path, out);
        } else if path.extension().is_some_and(|x| x == "css") {
            out.push(path);
        }
    }
}

/// Every binding declared in `lib/shortcutBindings.ts` must have a
/// handler, and every documented one must be reachable.
///
/// This closes both halves of a failure the codebase has now had in
/// each direction:
///
/// - **⌘F** was listed in the shortcuts modal and in `design.md` for a
///   long time while no section ever wired it. The hook accepted an
///   `onFilter` option and nothing passed one.
/// - **⌘\** was the mirror: bound in `useSidebarCollapsed` from the
///   day it was added, surfaced only in a tooltip, absent from the
///   modal and the rules file.
///
/// Both come from documentation and implementation being two lists.
/// The table is now one list; this asserts the code still matches it.
fn check_shortcut_table_has_handlers(repo: &Path, problems: &mut Vec<String>) -> Result<()> {
    let table = read(repo, "src/lib/shortcutBindings.ts")?;
    let table_code = strip_comments(&table);

    // `key: "x"` entries in the table.
    // `key: "x"` anywhere, not only at the start of a line. The first
    // draft used `line.trim().strip_prefix("key: \"")`, which only saw
    // the multi-line entries — 2 of the 9 bindings — so the check ran
    // over a quarter of the table and passed a deliberately planted
    // ⌘F. `labelKey: "` cannot collide: its `Key` is capitalised.
    let mut declared: Vec<String> = Vec::new();
    let mut from = 0usize;
    while let Some(rel) = table_code[from..].find("key: \"") {
        let at = from + rel;
        let after = at + "key: \"".len();
        // Reject `…someKey: "` — require a boundary before `key`.
        let ok = table_code[..at]
            .chars()
            .next_back()
            .is_none_or(|c| c.is_whitespace() || c == ',' || c == '{');
        if ok {
            if let Some(end) = table_code[after..].find('"') {
                declared.push(table_code[after..after + end].to_string());
            }
        }
        from = after;
    }
    if declared.is_empty() {
        problems.push(
            "lib/shortcutBindings.ts declares no `key:` entries — the handler check \
             cannot run, so it must not silently pass"
                .into(),
        );
        return Ok(());
    }

    // Scan ALL of src/, recursively. The first draft looked only in
    // src/hooks, src/components and src/shell and reported ⌘⇧C as
    // unimplemented — its handler lives in
    // src/sections/AccountsSection.tsx. Acting on that would have
    // deleted the documentation for a working feature, which is the
    // opposite of the mistake this check exists to catch.
    let mut handler_src = String::new();
    collect_ts_sources(&repo.join("src"), &mut handler_src)?;

    for key in declared {
        // Match on the COMPARISON, not on a bare string literal.
        // Single-letter needles like "c" or "r" occur constantly in a
        // recursive scan of the whole tree, so a bare-literal search
        // over src/ would pass everything and assert nothing.
        //
        // Shift reports an uppercase `e.key`, so handlers legitimately
        // compare either case; accept both.
        let upper = key.to_uppercase();
        let found = [key.as_str(), upper.as_str()].iter().any(|k| {
            [
                format!("key === \"{k}\""),
                format!("key !== \"{k}\""),
                format!("key === '{k}'"),
                format!("key !== '{k}'"),
            ]
            .iter()
            .any(|n| handler_src.contains(n.as_str()))
        });
        if !found {
            problems.push(format!(
                "shortcutBindings.ts declares key {key:?} but nothing under src/ compares \
                 `e.key` against it — documenting a shortcut that does nothing is the \
                 ⌘F mistake"
            ));
        }
    }
    Ok(())
}

/// Every non-test `.ts` / `.tsx` path under `dir`, recursively.
fn collect_ts_paths(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.filter_map(Result::ok) {
        let path = e.path();
        if path.is_dir() {
            collect_ts_paths(&path, out);
            continue;
        }
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        if name.contains(".test.") {
            continue;
        }
        if path.extension().is_some_and(|x| x == "ts" || x == "tsx") {
            out.push(path);
        }
    }
}

/// Append every non-test `.ts` / `.tsx` source under `dir`, comments
/// stripped, recursively.
fn collect_ts_sources(dir: &Path, out: &mut String) -> Result<()> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(());
    };
    for e in entries.filter_map(Result::ok) {
        let path = e.path();
        if path.is_dir() {
            collect_ts_sources(&path, out)?;
            continue;
        }
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        if name.contains(".test.") {
            continue;
        }
        if path.extension().is_some_and(|x| x == "ts" || x == "tsx") {
            if let Ok(src) = std::fs::read_to_string(&path) {
                out.push_str(&strip_comments(&src));
            }
        }
    }
    Ok(())
}

/// A `@media (prefers-contrast: more)` override must appear AFTER the
/// plain declaration it overrides.
///
/// A media query adds **no specificity**, so `:root` inside one and
/// `:root` outside it are equal and source order decides. The first
/// draft of the Increase-Contrast block sat above the legacy alias
/// section, where `--focus-ring` is declared — so the override lost
/// the cascade and did nothing. The CSS parsed, the build passed, and
/// the accessibility setting silently had no effect.
///
/// That is the failure this guards: not a syntax error, a no-op.
///
/// The first draft of *this check* was itself a no-op, for a related
/// reason worth recording: it decided "am I still inside the media
/// block" by counting braces from the block's start, so any later
/// `:root {` made the count unbalanced and every subsequent
/// declaration looked like it was inside. It reported green over a
/// deliberately planted defect. It now brace-matches each block to its
/// real end.
fn check_contrast_overrides_win(repo: &Path, problems: &mut Vec<String>) -> Result<()> {
    let src = read(repo, "src/styles/tokens.css")?;
    let b = src.as_bytes();

    // Locate every prefers-contrast block and its true end.
    let mut blocks: Vec<(usize, usize)> = Vec::new();
    let mut from = 0usize;
    while let Some(rel) = src[from..].find("@media (prefers-contrast") {
        let open_at = from + rel;
        let Some(brace) = src[open_at..].find('{').map(|i| open_at + i) else {
            break;
        };
        let mut depth = 0usize;
        let mut i = brace;
        while i < b.len() {
            match b[i] {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            i += 1;
        }
        blocks.push((open_at, i));
        from = i + 1;
    }

    if blocks.is_empty() {
        problems.push(
            "tokens.css has no `prefers-contrast: more` block — design.md's accessibility \
             floor commits to honouring it"
                .into(),
        );
        return Ok(());
    }

    // Tokens set inside those blocks, and the last block's end.
    let mut overridden: BTreeSet<String> = BTreeSet::new();
    for (s0, e0) in &blocks {
        overridden.extend(declared_properties(&src[*s0..*e0]));
    }
    let last_end = blocks.iter().map(|(_, e)| *e).max().unwrap_or(0);

    if overridden.is_empty() {
        problems
            .push("the prefers-contrast block sets no tokens — it cannot be doing anything".into());
        return Ok(());
    }

    // Any plain declaration of those tokens after the last block wins
    // the cascade and makes the override inert.
    for (n, line) in src[last_end..].lines().enumerate() {
        for token in declared_properties(line) {
            if overridden.contains(&token) {
                problems.push(format!(
                    "tokens.css re-declares {token} after the prefers-contrast block \
                 (line ~{}); a media query adds no specificity, so the accessibility \
                 override is inert",
                    src[..last_end].lines().count() + n
                ));
            }
        }
    }
    Ok(())
}

/// Every `keydown` listener under `src/hooks/` must reach the shared
/// shortcut gate rather than re-deriving it.
///
/// # Why this is a build gate and not a comment
///
/// `design.md` already says it in words — "The one predicate for that
/// is `isShortcutContextBlocked()` … use it rather than re-deriving
/// the check" — and `useGlobalShortcuts` carries a comment recording
/// what the last violation cost. The rule was then broken **twice
/// more**: `useSection` kept a behaviourally-equivalent copy, and
/// `useSidebarCollapsed` shipped a *weaker* one that tested editable
/// focus but never `[role="dialog"]`, so ⌘\ collapsed the sidebar out
/// from under an open modal.
///
/// A convention that has been violated twice after being written down
/// is not being enforced by being written down a third time.
///
/// Exempt: the file that defines the predicate, and hooks that touch
/// `activeElement` for a purpose other than gating a shortcut (focus
/// traps restore focus — they are not shortcut handlers, and they have
/// no keydown-with-modifier shape).
/// Remove `//` line comments and `/* */` block comments so a source
/// scan tests code rather than prose. Deliberately naive — it does not
/// parse strings — which is safe here because every caller is asking
/// "does this file CALL x", and a false positive from an identifier
/// inside a string literal fails closed (reports drift) rather than
/// open.
fn strip_comments(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let b = src.as_bytes();
    let mut i = 0usize;
    while i < b.len() {
        if b[i] == b'/' && i + 1 < b.len() && b[i + 1] == b'/' {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if b[i] == b'/' && i + 1 < b.len() && b[i + 1] == b'*' {
            i += 2;
            while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(b.len());
        } else {
            out.push(b[i] as char);
            i += 1;
        }
    }
    out
}

fn check_shortcut_gate_is_shared(repo: &Path, problems: &mut Vec<String>) -> Result<()> {
    let mut scanned = 0usize;
    let mut files: Vec<PathBuf> = Vec::new();
    collect_ts_paths(&repo.join("src"), &mut files);
    if files.is_empty() {
        problems.push("src/ is unreadable — the shortcut-gate check cannot run".into());
        return Ok(());
    }
    for path in files {
        {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            // The definition site cannot import itself.
            if name.starts_with("useGlobalShortcuts") || name.contains(".test.") {
                continue;
            }
            let src = std::fs::read_to_string(&path)?;
            let registers_keydown = src.contains("\"keydown\"") || src.contains("'keydown'");
            // A *shortcut* is modifier-keyed. A bare-key keydown handler
            // is something else and must not be swept in: `useFocusTrap`
            // listens for Tab and is required to keep working while a
            // modal is open, which is the exact opposite of what this
            // gate enforces. Keying on the modifier separates the two by
            // what they are rather than by a name allowlist.
            let is_shortcut = src.contains("metaKey") || src.contains("ctrlKey");
            if !registers_keydown || !is_shortcut {
                continue;
            }
            scanned += 1;
            // Comments are stripped before the check. Without this, the
            // doc comment *explaining* the rule satisfies it: the first
            // draft of this guard passed a deliberately reintroduced
            // ⌘\ bug because the file still carried a comment naming
            // `isShortcutContextBlocked`. A guard that a comment can
            // satisfy is worse than none — it reports green over the
            // exact regression it was written for.
            if !strip_comments(&src).contains("isShortcutContextBlocked") {
                let rel = path.strip_prefix(repo).unwrap_or(&path).display();
                problems.push(format!(
                    "{rel} registers a modifier-keyed keydown listener without importing \
                     isShortcutContextBlocked — re-deriving the gate is how ⌘\\ ended up \
                     firing under an open modal (rules/design.md)"
                ));
            }
        }
    }
    // A check that silently scanned nothing is indistinguishable from a
    // check that passed.
    if scanned == 0 {
        problems.push(
            "the shortcut-gate check matched no keydown listeners under src/hooks — \
             the scan is broken, not the codebase"
                .into(),
        );
    }
    Ok(())
}

/// The website's icon assets are hand-copied from `assets/icon-set/`,
/// and nothing regenerates them — `scripts/regen-icons.sh` covers the
/// app ladder only. This asserts they still track their masters.
///
/// # Why this exists
///
/// The v0.4.12 icon redesign updated `web/src/app/icon.svg` and
/// `apple-icon.png` and missed `web/public/claudepot-logo.svg`. For two
/// releases claudepot.com served the new mark in the browser tab and
/// the retired pixel house in its own nav — while the file it was
/// derived from had been deleted from the repo. Nothing failed,
/// because nothing was looking.
///
/// # The two checks are deliberately different shapes
///
/// The favicon is a **byte copy** of the flat master, so byte equality
/// is the honest assertion. The nav logo is not and cannot be: it
/// carries a provenance comment and a squared viewBox, because it is
/// composited onto a themed page rather than into a tab strip. Byte
/// equality there would force one of the two to be wrong. What must
/// hold is that the *artwork* matches — so this compares the block's
/// path geometry, which is what changes when the mark is redrawn.
fn check_web_icon_provenance(repo: &Path, problems: &mut Vec<String>) -> Result<()> {
    // Favicon: a straight copy of the plated flat master.
    let master = repo.join("assets/icon-set/app-icon-flat.svg");
    let favicon = repo.join("web/src/app/icon.svg");
    match (std::fs::read(&master), std::fs::read(&favicon)) {
        (Ok(a), Ok(b)) if a != b => problems.push(
            "web/src/app/icon.svg has drifted from assets/icon-set/app-icon-flat.svg \
             (the favicon is a byte copy of that master — re-copy it)"
                .to_string(),
        ),
        (Err(_), _) => problems.push("assets/icon-set/app-icon-flat.svg is missing".to_string()),
        (_, Err(_)) => problems.push("web/src/app/icon.svg is missing".to_string()),
        _ => {}
    }

    // Nav logo: same artwork, different framing — compare geometry.
    let glyph = repo.join("assets/icon-set/windows/icon-glyph.svg");
    let logo = repo.join("web/public/claudepot-logo.svg");
    let (Ok(glyph_src), Ok(logo_src)) = (
        std::fs::read_to_string(&glyph),
        std::fs::read_to_string(&logo),
    ) else {
        problems.push(
            "assets/icon-set/windows/icon-glyph.svg or web/public/claudepot-logo.svg is missing"
                .to_string(),
        );
        return Ok(());
    };

    let paths = |s: &str| -> Vec<String> {
        s.match_indices(" d=\"")
            .filter_map(|(i, m)| {
                let rest = &s[i + m.len()..];
                rest.find('"').map(|end| rest[..end].trim().to_string())
            })
            .collect()
    };
    let want = paths(&glyph_src);
    if want.is_empty() {
        problems.push(
            "assets/icon-set/windows/icon-glyph.svg has no <path d=…> — the logo check \
             cannot run, so it must not silently pass"
                .to_string(),
        );
        return Ok(());
    }
    let have = paths(&logo_src);
    for d in &want {
        if !have.contains(d) {
            problems.push(format!(
                "web/public/claudepot-logo.svg is missing a block face from \
                 assets/icon-set/windows/icon-glyph.svg (path starting `{}`) — re-derive it",
                d.chars().take(28).collect::<String>()
            ));
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

// ─── guard self-tests ────────────────────────────────────────────────
//
// Every check in this file reports a problem by NOT reporting one: a
// guard that has stopped working is indistinguishable from a clean
// repo. Six of the guards written for the 2026-08 UX-audit branch
// passed over a deliberately planted defect on their first draft —
// three were satisfied by a comment naming the thing they check for,
// one parsed 2 of 9 entries, one could never flag anything because it
// counted braces from the wrong origin, and one compared only the
// items that had not regressed. Each was found by hand-sabotaging the
// tree and watching for red.
//
// AGENTS.md already names this failure and prescribes the fix, for
// `check:envvar-layout`: "Split the judgement out of the measurement
// in any guard of this shape; the measurement may need a screen, the
// judgement never does." These guards are pure judgement over file
// contents, so the sabotage belongs in CI rather than in someone's
// memory. Each test below plants the exact defect its guard exists to
// catch and fails if the guard stays quiet — plus a clean-fixture
// counterpart, so a check that flags everything cannot pass either.
#[cfg(test)]
mod guard_tests {
    use super::*;
    use std::fs;

    /// Minimal synthetic repo. Only the files a given check reads need
    /// to exist; each test writes what it needs on top.
    fn repo() -> tempfile::TempDir {
        let d = tempfile::tempdir().expect("tempdir");
        for sub in ["src/hooks", "src/lib", "src/styles", "src/sections"] {
            fs::create_dir_all(d.path().join(sub)).unwrap();
        }
        d
    }

    fn write(d: &tempfile::TempDir, rel: &str, body: &str) {
        let p = d.path().join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, body).unwrap();
    }

    fn run(
        f: impl Fn(&Path, &mut Vec<String>) -> Result<()>,
        d: &tempfile::TempDir,
    ) -> Vec<String> {
        let mut problems = Vec::new();
        f(d.path(), &mut problems).expect("check ran");
        problems
    }

    // ── shortcut gate ────────────────────────────────────────────────

    const GATED: &str = r#"
        import { isShortcutContextBlocked } from "./useGlobalShortcuts";
        window.addEventListener("keydown", (e) => {
          if (!e.metaKey) return;
          if (isShortcutContextBlocked()) return;
        });
    "#;

    #[test]
    fn shortcut_gate_accepts_a_hook_using_the_shared_predicate() {
        let d = repo();
        write(&d, "src/hooks/useThing.ts", GATED);
        assert!(run(check_shortcut_gate_is_shared, &d).is_empty());
    }

    #[test]
    fn shortcut_gate_catches_a_local_re_derivation() {
        let d = repo();
        write(
            &d,
            "src/hooks/useThing.ts",
            r#"
            window.addEventListener("keydown", (e) => {
              if (!e.metaKey) return;
              if (document.activeElement) return;
            });
        "#,
        );
        assert_eq!(run(check_shortcut_gate_is_shared, &d).len(), 1);
    }

    /// The first draft was satisfied by a COMMENT naming the predicate,
    /// so it reported green over a reintroduced ⌘\ bug.
    #[test]
    fn shortcut_gate_is_not_satisfied_by_a_comment() {
        let d = repo();
        write(
            &d,
            "src/hooks/useThing.ts",
            r#"
            // Deliberately does not call isShortcutContextBlocked here.
            window.addEventListener("keydown", (e) => {
              if (!e.metaKey) return;
              if (document.activeElement) return;
            });
        "#,
        );
        assert_eq!(run(check_shortcut_gate_is_shared, &d).len(), 1);
    }

    /// Focus traps listen for bare Tab and MUST keep working while a
    /// modal is open — the opposite of what this gate enforces.
    #[test]
    fn shortcut_gate_ignores_non_modifier_handlers() {
        let d = repo();
        write(&d, "src/hooks/useThing.ts", GATED); // keeps the scan non-empty
        write(
            &d,
            "src/hooks/useFocusTrap.ts",
            r#"
            window.addEventListener("keydown", (e) => {
              if (e.key !== "Tab") return;
            });
        "#,
        );
        assert!(run(check_shortcut_gate_is_shared, &d).is_empty());
    }

    /// A scan that matches nothing must fail loudly, not pass.
    #[test]
    fn shortcut_gate_reports_when_it_scanned_nothing() {
        let d = repo();
        assert_eq!(run(check_shortcut_gate_is_shared, &d).len(), 1);
    }

    // ── contrast overrides ───────────────────────────────────────────

    const CONTRAST_OK: &str = r#"
        :root { --line: red; --focus-ring: 0 0 0 3px blue; }
        @media (prefers-contrast: more) {
          :root { --line: black; --focus-ring: 0 0 0 3px black; }
        }
    "#;

    #[test]
    fn contrast_accepts_overrides_declared_last() {
        let d = repo();
        write(&d, "src/styles/tokens.css", CONTRAST_OK);
        assert!(run(check_contrast_overrides_win, &d).is_empty());
    }

    /// A media query adds no specificity, so a later plain declaration
    /// wins and the accessibility override silently does nothing.
    #[test]
    fn contrast_catches_a_later_plain_redeclaration() {
        let d = repo();
        write(
            &d,
            "src/styles/tokens.css",
            &format!("{CONTRAST_OK}\n:root {{ --focus-ring: 0 0 0 3px blue; }}\n"),
        );
        let p = run(check_contrast_overrides_win, &d);
        assert_eq!(p.len(), 1, "{p:?}");
        assert!(p[0].contains("--focus-ring"));
    }

    #[test]
    fn contrast_reports_a_missing_block() {
        let d = repo();
        write(&d, "src/styles/tokens.css", ":root { --line: red; }");
        assert_eq!(run(check_contrast_overrides_win, &d).len(), 1);
    }

    // ── shortcut table ───────────────────────────────────────────────

    #[test]
    fn shortcut_table_accepts_a_binding_with_a_handler() {
        let d = repo();
        write(&d, "src/lib/shortcutBindings.ts", r#"{ key: "k" },"#);
        write(&d, "src/hooks/useShell.ts", r#"if (e.key === "k") open();"#);
        assert!(run(check_shortcut_table_has_handlers, &d).is_empty());
    }

    /// The ⌘F mistake: documented, never wired.
    #[test]
    fn shortcut_table_catches_a_binding_with_no_handler() {
        let d = repo();
        write(&d, "src/lib/shortcutBindings.ts", r#"{ key: "q" },"#);
        write(&d, "src/hooks/useShell.ts", r#"if (e.key === "k") open();"#);
        assert_eq!(run(check_shortcut_table_has_handlers, &d).len(), 1);
    }

    /// The first draft only read `key:` at the start of a line, so it
    /// parsed 2 of 9 single-line entries and asserted almost nothing.
    #[test]
    fn shortcut_table_parses_entries_that_share_a_line() {
        let d = repo();
        write(
            &d,
            "src/lib/shortcutBindings.ts",
            r#"export const A = [{ keys: ["x"], labelKey: "a", key: "q" }];"#,
        );
        write(&d, "src/hooks/useShell.ts", r#"if (e.key === "k") open();"#);
        assert_eq!(
            run(check_shortcut_table_has_handlers, &d).len(),
            1,
            "a single-line entry must still be parsed"
        );
    }

    /// Shift reports an uppercase `e.key`; comparing either case counts.
    #[test]
    fn shortcut_table_accepts_an_uppercase_comparison() {
        let d = repo();
        write(&d, "src/lib/shortcutBindings.ts", r#"{ key: "c" },"#);
        write(&d, "src/sections/S.tsx", r#"if (e.key === "C") copy();"#);
        assert!(run(check_shortcut_table_has_handlers, &d).is_empty());
    }

    // ── undefined tokens ─────────────────────────────────────────────

    #[test]
    fn tokens_accept_a_declared_reference() {
        let d = repo();
        write(&d, "src/styles/tokens.css", ":root { --fg: red; }");
        write(&d, "src/sections/S.tsx", r#"color: "var(--fg)""#);
        assert!(run(check_no_undefined_tokens, &d).is_empty());
    }

    /// A misspelled token is not a raw value, so the no-raw-values rule
    /// never sees it; CSS drops the declaration and the element inherits.
    #[test]
    fn tokens_catch_an_undeclared_reference() {
        let d = repo();
        write(&d, "src/styles/tokens.css", ":root { --fg: red; }");
        write(&d, "src/sections/S.tsx", r#"color: "var(--fg-2)""#);
        let p = run(check_no_undefined_tokens, &d);
        assert_eq!(p.len(), 1, "{p:?}");
        assert!(p[0].contains("--fg-2"));
    }

    /// `var(--x, fallback)` is legitimate even when `--x` is absent.
    #[test]
    fn tokens_allow_a_var_with_a_fallback() {
        let d = repo();
        write(&d, "src/styles/tokens.css", ":root { --fg: red; }");
        write(&d, "src/sections/S.tsx", r#"color: "var(--maybe, red)""#);
        assert!(run(check_no_undefined_tokens, &d).is_empty());
    }

    /// Locally-scoped properties are valid. The first draft reported
    /// `--rank-col` as undefined when it is set on the element that
    /// reads it — and its quote detection took the pattern's last
    /// character instead of its first, so no local ever matched.
    #[test]
    fn tokens_allow_a_locally_declared_property() {
        let d = repo();
        write(&d, "src/styles/tokens.css", ":root { --fg: red; }");
        write(
            &d,
            "src/sections/S.tsx",
            r#"style={{ ["--rank-col"]: "8px", width: "var(--rank-col)" }}"#,
        );
        assert!(run(check_no_undefined_tokens, &d).is_empty());
    }

    /// Three guards in one branch were fooled by their own docs.
    #[test]
    fn tokens_ignore_references_inside_comments() {
        let d = repo();
        write(
            &d,
            "src/styles/tokens.css",
            ":root { --fg: red; }\n/* was var(--sp-160), which does not exist */",
        );
        write(&d, "src/sections/S.tsx", r#"color: "var(--fg)""#);
        assert!(run(check_no_undefined_tokens, &d).is_empty());
    }
}
