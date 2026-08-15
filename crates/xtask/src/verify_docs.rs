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
    check_runtime_tokens_are_registered(repo, &mut problems)?;
    check_tokens_declared_only_in_tokens_css(repo, &mut problems)?;
    check_optional_shortcut_callbacks_are_wired(repo, &mut problems)?;
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
    declared_properties_at(src)
        .into_iter()
        .map(|(n, _)| n)
        .collect()
}

/// Every `--name:` declaration in `src`, with its byte offset.
///
/// Offsets exist because the contrast check needs to name a line, and
/// scanning line-by-line missed a declaration split across lines
/// (`--focus-ring\n  : blue;` is valid CSS) as well as one sharing a
/// line with the closing brace of the block it defeats.
fn declared_properties_at(src: &str) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    let b = src.as_bytes();
    let mut i = 0usize;
    while let Some(rel) = src[i..].find("--") {
        let at = i + rel;
        let name_start = at + 2;
        let mut j = name_start;
        while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'-' || b[j] == b'_') {
            j += 1;
        }
        if j > name_start {
            let mut k = j;
            while k < b.len() && (b[k] as char).is_whitespace() {
                k += 1;
            }
            if k < b.len() && b[k] == b':' {
                out.push((format!("--{}", &src[name_start..j]), at));
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
/// # Known limit — scope is flattened
///
/// A name declared anywhere in `tokens.css` counts as declared, so a
/// token defined only under `[data-theme="dark"]` satisfies a reference
/// used in light mode. Per-selector scope needs a cascade model; the
/// file's convention is that every token is declared on the base
/// `:root` and only overridden per theme, which is what makes the flat
/// check sound here.
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

    // NO local-property exemption. `design.md` says tokens.css is
    // "the one place tokens are declared. No other file opens a
    // `:root { }` block or redeclares `--*` custom properties." An
    // earlier draft exempted locally-declared properties to silence a
    // false positive on `--rank-col` — but per that rule `--rank-col`
    // WAS the violation, and the exemption legitimised it rather than
    // reporting it. Both call sites were removed instead, so the gate
    // is now exactly as strict as the rule it enforces.

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
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
                .map(|i| at + i)
                .unwrap_or(src.len());
            let name = &src[at..end];
            scanned += 1;
            // A fallback does NOT exempt the token. It hid four
            // genuinely undeclared names — `--danger-border`,
            // `--bg-warning-soft`, `--traffic-light-center-y` and
            // `--bad` (deleted by this branch's own codemod, whose
            // regex rewrote only the bare form). Each was a phantom
            // override hook: read with a fallback, declared nowhere,
            // so the fallback was the only value it ever took —
            // which reads in review as a configurable value and is
            // dead weight. A genuinely runtime-set property belongs
            // in tokens.css, so there is no exemption to carve.
            let referenced = src[end..].starts_with(')') || src[end..].starts_with(',');
            if referenced && !declared.contains(name) {
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

/// Unescape a TypeScript string literal's contents.
///
/// Both sides of the shortcut check read raw source, so both see
/// `\\` where the value is a single backslash — the ⌘\ binding. Applying
/// this to only one side made the two halves disagree about that one
/// key, which is precisely the binding the reverse check is named
/// after.
fn unescape_ts(raw: &str) -> String {
    raw.replace("\\\\", "\\")
        .replace("\\\"", "\"")
        .replace("\\'", "'")
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

    let declared = parse_shortcut_table(&table_code, problems);
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
    let mut handler_files: Vec<PathBuf> = Vec::new();
    collect_ts_paths(&repo.join("src"), &mut handler_files);
    let mut handled: BTreeSet<Binding> = BTreeSet::new();
    for path in handler_files {
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let rel = path
            .strip_prefix(repo)
            .unwrap_or(&path)
            .display()
            .to_string();
        collect_handled_bindings(&rel, &strip_comments(&raw), &mut handled, problems);
    }

    // Both directions compare the SAME tuple, produced by the same
    // parser. They used to extract keys differently — the table side
    // read the raw literal and the handler side unescaped it — so `\\`
    // and `\` never matched and ⌘\, the binding this check is named
    // after, was simultaneously reported missing in both directions.
    for b in &declared {
        if handled.contains(b) {
            continue;
        }
        problems.push(format!(
            "shortcutBindings.ts declares {} but no handler under src/ compares `e.key` \
             against it under those modifiers — documenting a shortcut that does nothing \
             is the ⌘F mistake",
            b.describe()
        ));
    }
    for b in &handled {
        if declared.contains(b)
            || UNDOCUMENTED_BY_DESIGN
                .iter()
                .any(|(x, _)| *x == b.describe())
        {
            continue;
        }
        problems.push(format!(
            "a modifier-keyed handler binds {} but no entry in lib/shortcutBindings.ts \
             declares it — an undocumented working shortcut is the ⌘\\ mistake, the \
             mirror of the ⌘F one",
            b.describe()
        ));
    }
    // Validated in both directions, like UNSUBSCRIBED_BY_DESIGN: an
    // exemption whose handler is gone is a stale rationale, and the
    // next reader would take it as evidence the shortcut still exists.
    for (combo, why) in UNDOCUMENTED_BY_DESIGN {
        if !handled.iter().any(|b| b.describe() == *combo) {
            problems.push(format!(
                "UNDOCUMENTED_BY_DESIGN still exempts {combo} ({why}) but no handler binds \
                 it — delete the entry rather than leaving a rationale for nothing"
            ));
        }
    }

    Ok(())
}

/// A key plus the modifiers beyond ⌘/⌃ that must be held.
///
/// Shift and Alt are what separate two bindings on the same letter.
/// Matching the bare key made ⌘⇧L indistinguishable from ⌃⌥⌘L, so
/// deleting the ⌘⇧L handler left this check green — the other L
/// answered for it.
#[derive(PartialEq, Eq, PartialOrd, Ord, Debug)]
struct Binding {
    key: String,
    ctrl: bool,
    shift: bool,
    alt: bool,
}

impl Binding {
    fn describe(&self) -> String {
        let mut s = String::new();
        if self.ctrl {
            s.push('⌃');
        }
        if self.alt {
            s.push('⌥');
        }
        if self.shift {
            s.push('⇧');
        }
        s.push('⌘');
        s.push_str(&self.key);
        s
    }
}

/// Shortcuts that work but are deliberately absent from the table,
/// with the reason. Same contract as `UNSUBSCRIBED_BY_DESIGN`.
const UNDOCUMENTED_BY_DESIGN: &[(&str, &str)] = &[(
    "⌃⌥⌘l",
    "developer-mode toggle: rules/design.md makes it the one ungated \
     shortcut precisely because it has no visible control, so listing it \
     in the user-facing shortcuts modal would contradict the rule",
)];

/// `{ keys: [...], key: "x" }` entries from the bindings table.
fn parse_shortcut_table(code: &str, problems: &mut Vec<String>) -> BTreeSet<Binding> {
    let mut out = BTreeSet::new();
    let b = code.as_bytes();
    let mut i = 0usize;
    while i < b.len() {
        if b[i] != b'{' {
            i += 1;
            continue;
        }
        // Brace-match the object. Splitting on `keys: [` assumed the
        // modifier list came first; `{ key: "q", keys: [...] }` is
        // equally valid TypeScript and was silently skipped, so a
        // documented-but-dead binding written that way passed.
        let mut depth = 0usize;
        let mut j = i;
        while j < b.len() {
            match b[j] {
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            j += 1;
        }
        let obj = &code[i..j.min(code.len())];
        i = j + 1;
        let Some(key) = quoted_after(obj, "key: \"") else {
            continue;
        };
        let key = unescape_ts(&key).to_lowercase();
        let Some(open) = obj.find("keys: [") else {
            continue;
        };
        let Some(close) = obj[open..].find(']').map(|x| open + x) else {
            continue;
        };
        let keys = &obj[open..close];
        // The chips the modal renders and the key the handler compares
        // must be the same key. They are separate fields, so changing
        // one advertises a shortcut that does not exist.
        if let Some(last) = keys
            .split(',')
            .filter_map(|part| {
                let t = part
                    .trim()
                    .trim_matches(|c| c == '"' || c == '[' || c == '\'');
                (!t.is_empty()).then(|| t.to_string())
            })
            .next_back()
        {
            let last = unescape_ts(&last).to_lowercase();
            if last.chars().count() == 1 && last != key {
                problems.push(format!(
                    "shortcutBindings.ts renders {last:?} as the final chip but the handler \
                     compares {key:?} — the modal would advertise a shortcut nobody wired"
                ));
            }
        }
        out.insert(Binding {
            key,
            ctrl: keys.contains('⌃'),
            shift: keys.contains('⇧'),
            alt: keys.contains('⌥'),
        });
    }
    out
}

/// The contents of the first `"…"` following `needle`.
fn quoted_after(src: &str, needle: &str) -> Option<String> {
    let at = src.find(needle)? + needle.len();
    let end = src[at..].find('"')?;
    Some(src[at..at + end].to_string())
}

/// Modifier-keyed `e.key` comparisons in one file's source.
///
/// Handlers in this codebase share a shape: an early-return guard
/// naming the modifiers, then the key comparison, inside one
/// `(e: KeyboardEvent) =>` closure. The nearest preceding
/// `KeyboardEvent)` bounds the guard that applies — matching the bare
/// word `KeyboardEvent` instead let a later type annotation
/// (`const typed: KeyboardEvent = e`) restart the scope mid-closure and
/// hide every comparison after it.
///
/// # What it deliberately refuses to guess
///
/// The inference is lexical. Where a handler expresses its modifiers in
/// a form this cannot read — `e.shiftKey === false`, a destructured
/// `const { shiftKey } = e`, `getModifierState` — the file is REPORTED,
/// not silently assigned a modifier set. Guessing produced confident
/// wrong answers: `if (e.shiftKey && e.key === "k")` reads as a bare ⌘K
/// under a naive scan, so a table row for ⌘K would validate a handler
/// that only ever fires on ⌘⇧K.
///
/// # Known limit
///
/// A comparison in a helper function that takes the event but not the
/// guard is invisible here, because the modifiers are not in scope to
/// read. Keep the comparison in the guarded closure.
fn collect_handled_bindings(
    path: &str,
    src: &str,
    out: &mut BTreeSet<Binding>,
    problems: &mut Vec<String>,
) {
    if !src.contains("metaKey") && !src.contains("ctrlKey") {
        return;
    }
    let mut sites: Vec<(usize, String)> = Vec::new();
    for pat in ["key === \"", "key !== \"", "case \""] {
        let mut from = 0usize;
        while let Some(rel) = src[from..].find(pat) {
            let at = from + rel + pat.len();
            let Some(e) = src[at..].find('"') else { break };
            let k = unescape_ts(&src[at..at + e]);
            // `case "x":` only counts inside a `switch (e.key)`.
            let ok = pat != "case \""
                || src[..at]
                    .rfind("switch")
                    .is_some_and(|sw| src[sw..at].contains("e.key"));
            if ok && k.chars().count() == 1 {
                sites.push((at, k));
            }
            from = at + e;
        }
    }

    for (at, k) in sites {
        let scope = &src[src[..at].rfind("KeyboardEvent)").unwrap_or(0)..at];
        // A guard that REJECTS modifiers is not a shortcut. Requiring
        // only that the scope mentions metaKey let
        // `if (e.metaKey || e.ctrlKey) return;` — the opposite of a
        // shortcut — answer for a documented binding.
        let requires_mod = [
            "!mod",
            "!e.metaKey",
            "!e.ctrlKey",
            // `!(e.metaKey || e.ctrlKey)` — ConfigSection's ⌘F spells
            // it this way, and omitting the form reported a live
            // shortcut as unwired.
            "!(e.metaKey",
            "!(e.ctrlKey",
            "e.metaKey &&",
            "e.ctrlKey &&",
        ]
        .iter()
        .any(|n| scope.contains(n));
        if !requires_mod {
            continue;
        }
        let unreadable = [
            "shiftKey ===",
            "altKey ===",
            "metaKey ===",
            "ctrlKey ===",
            "getModifierState",
            "{ shiftKey",
            "{ altKey",
            "{ metaKey",
            "{ ctrlKey",
        ]
        .iter()
        .find(|n| scope.contains(**n));
        if let Some(form) = unreadable {
            problems.push(format!(
                "{path} guards a shortcut with {form:?}, which the modifier check cannot \
                 read — it would assign this binding a modifier set by guessing. Use the \
                 early-return form (`if (!mod || e.shiftKey) return;`) or extend the check"
            ));
            continue;
        }
        // In an `if (…) return;` guard the sign is inverted: `!e.altKey`
        // disqualifies the event unless Alt is held, so it REQUIRES Alt,
        // while a bare `e.altKey` forbids it. A positive `&&` conjunction
        // requires it directly.
        let req =
            |m: &str| scope.contains(&format!("!e.{m}")) || scope.contains(&format!("e.{m} &&"));
        out.insert(Binding {
            key: k.to_lowercase(),
            ctrl: req("ctrlKey"),
            shift: req("shiftKey"),
            alt: req("altKey"),
        });
    }
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

/// `tokens.css` is the only place a custom property is declared.
///
/// `rules/design.md` says so in words — "No other file opens a
/// `:root { }` block or redeclares `--*` custom properties" — and
/// nothing enforced it. The rule had already been broken once:
/// `--rank-col` was declared inline on a grid container and read two
/// lines later, and the token check grew a "locally declared" exemption
/// to stay quiet about it. Removing that exemption fixed the instance;
/// this closes the class, so the next one cannot arrive unnoticed.
///
/// Runtime writes through `style.setProperty` are a different mechanism
/// with its own check — they must name a token `tokens.css` declares,
/// which is the opposite of declaring one elsewhere.
fn check_tokens_declared_only_in_tokens_css(repo: &Path, problems: &mut Vec<String>) -> Result<()> {
    let tokens_css = repo.join("src/styles/tokens.css");
    let mut css: Vec<PathBuf> = Vec::new();
    collect_css_paths(&repo.join("src"), &mut css);
    for path in css {
        if path == tokens_css {
            continue;
        }
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let rel = path
            .strip_prefix(repo)
            .unwrap_or(&path)
            .display()
            .to_string();
        for (name, _) in declared_properties_at(&strip_comments(&raw)) {
            problems.push(format!(
                "{rel} declares {name} — tokens.css is the one declaration site \
                 (rules/design.md). Add a semantic token there and reference it"
            ));
        }
    }

    let mut ts: Vec<PathBuf> = Vec::new();
    collect_ts_paths(&repo.join("src"), &mut ts);
    for path in ts {
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        let src = strip_comments(&raw);
        let rel = path
            .strip_prefix(repo)
            .unwrap_or(&path)
            .display()
            .to_string();
        // `["--x" as keyof React.CSSProperties]: …` — an inline style
        // object declaring a property. `setProperty("--x", …)` is the
        // runtime channel and is checked elsewhere.
        let mut from = 0usize;
        while let Some(at) = src[from..].find("[\"--").map(|i| from + i) {
            from = at + 4;
            let Some(end) = src[at + 2..].find('"').map(|i| at + 2 + i) else {
                break;
            };
            let name = &src[at + 2..end];
            let after = src[end + 1..].trim_start();
            let closes = after.find(']').is_some_and(|b| {
                src[end + 1..end + 1 + b].trim_end().is_empty() || after.starts_with("as ")
            });
            if closes && src[end..].contains("]:") {
                problems.push(format!(
                    "{rel} declares {name} in an inline style — tokens.css is the one \
                     declaration site (rules/design.md)"
                ));
            }
        }
    }
    Ok(())
}

/// A custom property written at RUNTIME must be declared in
/// `tokens.css` and read by something.
///
/// This exists because of a regression it would have caught. A token
/// sweep replaced `var(--traffic-light-center-y, calc(…))` with its
/// fallback, on the reasoning that the token was undeclared and the
/// fallback was therefore the effective value. It was not:
/// `lib/trafficLights.ts` writes that property from AppKit at runtime,
/// so on macOS the fallback was the value that never applied. The
/// substitution compiled, rendered, and pinned the whole window chrome
/// to a constant offset of zero.
///
/// Substituting a fallback is behaviour-preserving only when nothing
/// sets the token at runtime, and an undeclared token gives a reviewer
/// no way to tell those apart. Registering the channel in `tokens.css`
/// is what makes the difference visible — the sibling
/// `--chrome-inset-left` was already handled that way, which is why the
/// sweep left it alone.
///
/// The reader half matters too: a runtime write nobody reads does
/// nothing at all, and is indistinguishable from one whose consumer was
/// deleted by accident. That is exactly the shape of the regression.
fn check_runtime_tokens_are_registered(repo: &Path, problems: &mut Vec<String>) -> Result<()> {
    let tokens = read(repo, "src/styles/tokens.css")?;
    let declared = declared_properties(&strip_comments(&tokens));

    let mut ts: Vec<PathBuf> = Vec::new();
    collect_ts_paths(&repo.join("src"), &mut ts);
    let mut css: Vec<PathBuf> = Vec::new();
    collect_css_paths(&repo.join("src"), &mut css);

    let mut all = String::new();
    for p in ts.iter().chain(css.iter()) {
        if let Ok(s) = std::fs::read_to_string(p) {
            all.push_str(&strip_comments(&s));
            all.push('\n');
        }
    }

    for path in &ts {
        let Ok(raw) = std::fs::read_to_string(path) else {
            continue;
        };
        let src = strip_comments(&raw);
        for open in ["setProperty(\"", "setProperty('"] {
            let quote = open.chars().next_back().unwrap();
            let mut from = 0usize;
            while let Some(rel) = src[from..].find(open) {
                let at = from + rel + open.len();
                let Some(e) = src[at..].find(quote) else {
                    break;
                };
                let name = src[at..at + e].to_string();
                from = at + e;
                if !name.starts_with("--") {
                    continue;
                }
                let rel_path = path.strip_prefix(repo).unwrap_or(path).display();
                if !declared.contains(&name) {
                    problems.push(format!(
                        "{rel_path} writes {name} at runtime but tokens.css never declares \
                         it — an unregistered channel reads as a typo, and its only \
                         consumer was once deleted on exactly that reasoning"
                    ));
                }
                if !all.contains(&format!("var({name}")) {
                    problems.push(format!(
                        "{rel_path} writes {name} at runtime but nothing reads it — a \
                         runtime write with no consumer does nothing, and looks identical \
                         to one whose consumer was deleted by mistake"
                    ));
                }
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
/// # Known limit — source order only
///
/// Source order is not the whole cascade. An earlier `!important`, or a
/// declaration on a higher-specificity selector, beats a later one and
/// makes the override inert without this noticing. Modelling that means
/// modelling CSS specificity, which is a different tool; `tokens.css`
/// declares everything on `:root` without `!important`, so within this
/// file order does decide. Reach for a real CSS parser if that stops
/// being true.
///
/// The first draft of *this check* was itself a no-op, for a related
/// reason worth recording: it decided "am I still inside the media
/// block" by counting braces from the block's start, so any later
/// `:root {` made the count unbalanced and every subsequent
/// declaration looked like it was inside. It reported green over a
/// deliberately planted defect. It now brace-matches each block to its
/// real end.
fn check_contrast_overrides_win(repo: &Path, problems: &mut Vec<String>) -> Result<()> {
    let raw = read(repo, "src/styles/tokens.css")?;
    // Comments are not CSS. Reading the raw file meant a block that had
    // been commented OUT still counted as present, so the check could
    // not notice the accessibility support being disabled wholesale.
    let src = strip_comments(&raw);

    let blocks = contrast_blocks(&src);
    if blocks.is_empty() {
        problems.push(
            "tokens.css has no live `prefers-contrast: more` block — design.md's \
             accessibility floor commits to honouring it"
                .into(),
        );
        return Ok(());
    }

    let sets: Vec<BTreeSet<String>> = blocks
        .iter()
        .map(|(s0, e0)| declared_properties(&src[*s0..*e0]))
        .collect();
    if sets.iter().all(BTreeSet::is_empty) {
        problems
            .push("the prefers-contrast block sets no tokens — it cannot be doing anything".into());
        return Ok(());
    }

    // Per block, not against the last one only: with two blocks (light
    // and system-dark) a plain re-declaration sitting BETWEEN them
    // defeats the first while passing an "after the last block" scan.
    for (idx, (_, end)) in blocks.iter().enumerate() {
        for (token, at) in declared_properties_at(&src[*end..]) {
            let at = end + at;
            if !sets[idx].contains(&token) {
                continue;
            }
            // Inside another contrast block this is an override, not a
            // plain declaration that out-orders ours.
            if blocks.iter().any(|(a, b)| at >= *a && at <= *b) {
                continue;
            }
            // A LATER contrast block that sets the same token restores
            // the override, so this one is not actually defeated.
            if blocks
                .iter()
                .enumerate()
                .any(|(k, (a, _))| k > idx && *a > at && sets[k].contains(&token))
            {
                continue;
            }
            problems.push(format!(
                "tokens.css re-declares {token} at line ~{} — after the prefers-contrast \
                 block that overrides it. A media query adds no specificity, so the \
                 accessibility override is inert",
                src[..at].lines().count()
            ));
        }
    }
    Ok(())
}

/// Live `@media (prefers-contrast: more)` blocks, as (start, end) byte
/// offsets.
///
/// Matching the bare prefix `@media (prefers-contrast` accepted
/// `no-preference`, which is a different — and in places opposite —
/// preference, so a file could satisfy the check while never
/// responding to Increase Contrast at all. A negated query is not the
/// block either.
fn contrast_blocks(src: &str) -> Vec<(usize, usize)> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut from = 0usize;
    while let Some(rel) = src[from..].find("prefers-contrast") {
        let hit = from + rel;
        from = hit + "prefers-contrast".len();
        let after = &src[from..];
        let Some(colon) = after.find(':') else {
            continue;
        };
        let Some(close) = after.find(')') else {
            continue;
        };
        if colon > close || after[colon + 1..close].trim() != "more" {
            continue; // `no-preference`, or not a value we honour
        }
        let Some(at) = src[..hit].rfind("@media") else {
            continue;
        };
        if src[at..hit].contains("not ") {
            continue; // `@media not (prefers-contrast: more)`
        }
        let Some(brace) = src[hit..].find('{').map(|i| hit + i) else {
            continue;
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
        out.push((at, i));
        from = i + 1;
    }
    out
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
    // Iterate CHARACTERS, not bytes. The first draft pushed
    // `b[i] as char`, which shreds every multibyte character into three
    // Latin-1 ones — so `⇧`, `⌥` and `⌘` never survived, and the
    // shortcut table's modifier columns read as empty.
    //
    // Two more things are NOT comments, and treating them as one
    // silently deletes live code to end of line:
    //   - `//` inside a string literal;
    //   - `//` in a URL — `url(https://x)` hid every token reference
    //     that followed it on the line, which is a false negative in a
    //     check whose whole job is to notice a missing token.
    let mut out = String::with_capacity(src.len());
    let mut it = src.chars().peekable();
    let mut prev = '\0';
    let mut quote: Option<char> = None;
    while let Some(c) = it.next() {
        if let Some(q) = quote {
            out.push(c);
            if c == q && prev != '\\' {
                quote = None;
            }
            prev = if c == '\\' && prev == '\\' { '\0' } else { c };
            continue;
        }
        if c == '"' || c == '\'' || c == '`' {
            quote = Some(c);
            out.push(c);
            prev = c;
            continue;
        }
        if c == '/' && prev != ':' {
            match it.peek() {
                Some('/') => {
                    // Keep the newline: other checks are line-oriented.
                    for c in it.by_ref() {
                        if c == '\n' {
                            out.push('\n');
                            break;
                        }
                    }
                    prev = '\n';
                    continue;
                }
                Some('*') => {
                    it.next();
                    let mut p2 = '\0';
                    for c in it.by_ref() {
                        if p2 == '*' && c == '/' {
                            break;
                        }
                        p2 = c;
                    }
                    prev = ' ';
                    out.push(' ');
                    continue;
                }
                _ => {}
            }
        }
        out.push(c);
        prev = c;
    }
    out
}

/// A shortcut gated on an optional callback must have somebody passing
/// that callback.
///
/// This is the ⌘F bug in its original form. The binding was documented
/// for years, the hook accepted an `onFilter` option, the handler
/// compared the key — and no caller ever passed one, so the shortcut
/// did nothing. Checking that a handler exists cannot see this: the
/// handler DOES exist, it just returns early forever.
///
/// `if (e.key === "n" && onAdd)` is the shape. If `onAdd` appears
/// nowhere outside the file that declares it, nothing can ever supply
/// it.
fn check_optional_shortcut_callbacks_are_wired(
    repo: &Path,
    problems: &mut Vec<String>,
) -> Result<()> {
    let mut files: Vec<PathBuf> = Vec::new();
    collect_ts_paths(&repo.join("src"), &mut files);
    let mut sources: Vec<(PathBuf, String)> = Vec::new();
    for path in &files {
        if let Ok(raw) = std::fs::read_to_string(path) {
            sources.push((path.clone(), strip_comments(&raw)));
        }
    }

    for (path, src) in &sources {
        let rel = path
            .strip_prefix(repo)
            .unwrap_or(path)
            .display()
            .to_string();
        let hooks = exported_hook_names(src);
        for (_, body) in keyboard_closures(src) {
            let body = blank_string_literals(&body);
            let mut from = 0usize;
            while let Some(rel_at) = body[from..].find("&& on") {
                let at = from + rel_at + "&& ".len();
                let end = at
                    + body[at..]
                        .find(|c: char| !c.is_ascii_alphanumeric())
                        .unwrap_or(body.len() - at);
                let ident = &body[at..end];
                from = end;
                // `on` + a capital: a callback prop, not `once`.
                if ident.len() < 3 || !ident[2..3].chars().all(|c| c.is_ascii_uppercase()) {
                    continue;
                }
                // Must appear in an ARGUMENT to the hook that declares
                // it. Mere presence somewhere else under src/ is not
                // wiring: `onAdd` is also an unrelated prop on a modal
                // in the same tree, so a presence test called the
                // binding wired after its only real call site was
                // deleted.
                let wired = sources.iter().any(|(p2, s2)| {
                    p2 != path
                        && hooks
                            .iter()
                            .any(|h| call_arguments(s2, h).iter().any(|a| a.contains(ident)))
                });
                if !wired {
                    problems.push(format!(
                        "{rel}: a shortcut is gated on `{ident}`, which no call site of \
                         {hooks:?} ever passes — the handler runs and returns, so the \
                         binding is dead. This is the ⌘F bug: documented, wired, inert"
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Names of exported hooks (`export function useX` / `export const useX =`).
fn exported_hook_names(src: &str) -> Vec<String> {
    let mut out = Vec::new();
    for pat in ["export function use", "export const use"] {
        let mut from = 0usize;
        while let Some(rel) = src[from..].find(pat) {
            let at = from + rel + "export function ".len() - "function ".len() + pat.len()
                - (pat.len() - "use".len())
                - "use".len();
            let start = from + rel + pat.len() - "use".len();
            let _ = at;
            let end = start
                + src[start..]
                    .find(|c: char| !c.is_ascii_alphanumeric())
                    .unwrap_or(0);
            if end > start {
                out.push(src[start..end].to_string());
            }
            from = start + 3;
        }
    }
    out.sort();
    out.dedup();
    out
}

/// The argument text of every `name(...)` call in `src`.
fn call_arguments(src: &str, name: &str) -> Vec<String> {
    let b = src.as_bytes();
    let needle = format!("{name}(");
    let mut out = Vec::new();
    let mut from = 0usize;
    while let Some(rel) = src[from..].find(&needle) {
        let open = from + rel + needle.len() - 1;
        let mut depth = 0usize;
        let mut i = open;
        while i < b.len() {
            match b[i] {
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            i += 1;
        }
        out.push(src[open..i.min(src.len())].to_string());
        from = i.max(open + 1);
    }
    out
}

/// Replace the CONTENTS of string literals with spaces.
///
/// `strip_comments` deliberately keeps strings — the token and shortcut
/// checks read key literals out of them. The gate check wants the
/// opposite: a diagnostic string that happens to contain
/// `isShortcutContextBlocked()` is not a call, and accepting one leaves
/// the listener ungated while the check reports green.
fn blank_string_literals(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut it = src.chars().peekable();
    let mut quote: Option<char> = None;
    let mut escaped = false;
    while let Some(c) = it.next() {
        match quote {
            Some(q) => {
                if escaped {
                    escaped = false;
                    out.push(' ');
                    continue;
                }
                if c == '\\' {
                    escaped = true;
                    out.push(' ');
                    continue;
                }
                out.push(if c == q { c } else { ' ' });
                if c == q {
                    quote = None;
                }
            }
            None => {
                if c == '"' || c == '\'' || c == '`' {
                    quote = Some(c);
                }
                out.push(c);
                let _ = it.peek();
            }
        }
    }
    out
}

/// Every modifier-keyed keydown handler must reach the shared shortcut
/// gate rather than re-deriving it.
///
/// # Why this is a build gate and not a comment
///
/// `design.md` already says it in words — "The one predicate for that
/// is `isShortcutContextBlocked()` … use it rather than re-deriving the
/// check" — and the newest shortcut in the app re-derived it anyway,
/// testing editable focus but never `[role="dialog"]`, so ⌘\ collapsed
/// the sidebar out from under an open modal.
///
/// # Per handler, not per file
///
/// The first version asked whether the FILE mentioned the predicate.
/// `useShellShortcuts` registers four separate listeners; one gated
/// handler licensed the other three, so deleting the Boards handler's
/// gate changed nothing. Scope is now the individual closure.
fn check_shortcut_gate_is_shared(repo: &Path, problems: &mut Vec<String>) -> Result<()> {
    let mut scanned = 0usize;
    let mut seen_exempt: BTreeSet<&str> = BTreeSet::new();
    let mut files: Vec<PathBuf> = Vec::new();
    collect_ts_paths(&repo.join("src"), &mut files);
    if files.is_empty() {
        problems.push("src/ is unreadable — the shortcut-gate check cannot run".into());
        return Ok(());
    }
    for path in &files {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        if name.contains(".test.") {
            continue;
        }
        // Comments are stripped first. Without this the doc comment
        // *explaining* the rule satisfies it: the first draft passed a
        // deliberately reintroduced ⌘\ bug because the file still
        // carried a comment naming the predicate.
        let src = strip_comments(&std::fs::read_to_string(path)?);
        let rel = path
            .strip_prefix(repo)
            .unwrap_or(path)
            .display()
            .to_string();
        for (handler, body) in keyboard_closures(&src) {
            let body = blank_string_literals(&body);
            // A *shortcut* is modifier-keyed. A bare-key keydown handler
            // is something else and must not be swept in: `useFocusTrap`
            // listens for Tab and is required to keep working while a
            // modal is open, which is the exact opposite of what this
            // gate enforces.
            let modifier_keyed = [
                "!mod",
                "!e.metaKey",
                "!e.ctrlKey",
                "!(e.metaKey",
                "!(e.ctrlKey",
            ]
            .iter()
            .any(|n| body.contains(n));
            if !modifier_keyed {
                continue;
            }
            // Counted before the exemption: the scan DID find a
            // modifier-keyed handler here, and the `scanned == 0`
            // sentinel exists to catch a scan that matched nothing at
            // all, not one whose only match was deliberately skipped.
            scanned += 1;
            if let Some((h, _)) = UNGATED_BY_DESIGN.iter().find(|(h, _)| *h == handler) {
                seen_exempt.insert(h);
                continue;
            }
            // Require the CALL, not the identifier: `contains` alone is
            // satisfied by an unused import, a string literal, or a
            // local stub, all of which leave the listener ungated.
            // Whitespace-tolerant: `isShortcutContextBlocked ()` is the
            // same call, and a stripped inline comment leaves a space.
            let called = body
                .split("isShortcutContextBlocked")
                .skip(1)
                .any(|rest| rest.trim_start().starts_with('('));
            if !called {
                problems.push(format!(
                    "{rel}: the `{handler}` keydown handler is modifier-keyed but never \
                     calls isShortcutContextBlocked() — re-deriving the gate is how ⌘\\ \
                     ended up firing under an open modal (rules/design.md)"
                ));
            }
        }
    }
    for (handler, why) in UNGATED_BY_DESIGN {
        if !seen_exempt.contains(handler) {
            problems.push(format!(
                "UNGATED_BY_DESIGN still exempts `{handler}` ({why}) but no such \
                 modifier-keyed handler exists — delete the entry rather than leaving a \
                 rationale for nothing"
            ));
        }
    }
    // A check that silently scanned nothing is indistinguishable from a
    // check that passed.
    if scanned == 0 {
        problems.push(
            "the shortcut-gate check matched no modifier-keyed keydown handlers under \
             src/ — the scan is broken, not the codebase"
                .into(),
        );
    }
    Ok(())
}

/// Handlers that are deliberately ungated, with the reason. Same
/// both-directions contract as `UNSUBSCRIBED_BY_DESIGN`.
const UNGATED_BY_DESIGN: &[(&str, &str)] = &[(
    "onDevKey",
    "⌃⌥⌘L toggles developer mode and rules/design.md makes it the sole \
     ungated shortcut on purpose: its value is being reachable exactly \
     when the UI is misbehaving, including from a modal that will not \
     dismiss",
)];

/// Each `(e: KeyboardEvent) => { … }` closure as (name, body).
///
/// The name is the `const <name> =` binding it is assigned to, which is
/// how this codebase writes every one of them; an anonymous closure
/// reports as `<anonymous>` and is still checked.
fn keyboard_closures(src: &str) -> Vec<(String, String)> {
    let b = src.as_bytes();
    let mut out = Vec::new();
    let mut from = 0usize;
    while let Some(rel) = src[from..].find("KeyboardEvent)") {
        let at = from + rel;
        from = at + "KeyboardEvent)".len();
        let Some(open) = src[at..].find('{').map(|i| at + i) else {
            break;
        };
        let mut depth = 0usize;
        let mut i = open;
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
        let head = &src[..at];
        let name = head
            .rfind("const ")
            .map(|c| {
                head[c + 6..]
                    .split(['=', ' ', ':'])
                    .next()
                    .unwrap_or("")
                    .trim()
            })
            .filter(|n| !n.is_empty())
            .unwrap_or("<anonymous>")
            .to_string();
        out.push((name, src[open..i.min(src.len())].to_string()));
        from = i.max(from);
    }
    out
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
        const onKey = (e: KeyboardEvent) => {
          if (!e.metaKey) return;
          if (isShortcutContextBlocked()) return;
        };
        window.addEventListener("keydown", onKey);
    "#;

    /// The handler `UNGATED_BY_DESIGN` exempts. That list is validated
    /// in both directions, so a fixture without it legitimately reports
    /// the exemption as stale.
    const EXEMPT: &str = r#"
        const onDevKey = (e: KeyboardEvent) => {
          if (!e.metaKey || !e.ctrlKey || !e.altKey) return;
          if (e.key !== "l") return;
        };
    "#;

    fn gate_repo(body: &str) -> tempfile::TempDir {
        let d = repo();
        write(&d, "src/hooks/useThing.ts", body);
        write(&d, "src/hooks/useDevToggle.ts", EXEMPT);
        d
    }

    #[test]
    fn shortcut_gate_accepts_a_hook_using_the_shared_predicate() {
        let d = gate_repo(GATED);
        assert_eq!(run(check_shortcut_gate_is_shared, &d), Vec::<String>::new());
    }

    #[test]
    fn shortcut_gate_catches_a_local_re_derivation() {
        let d = gate_repo(
            r#"
            const onKey = (e: KeyboardEvent) => {
              if (!e.metaKey) return;
              if (document.activeElement) return;
            };
            window.addEventListener("keydown", onKey);
        "#,
        );
        let out = run(check_shortcut_gate_is_shared, &d);
        assert_eq!(out.len(), 1, "{out:?}");
    }

    /// The first draft was satisfied by a COMMENT naming the predicate,
    /// so it reported green over a reintroduced ⌘\ bug.
    #[test]
    fn shortcut_gate_is_not_satisfied_by_a_comment() {
        let d = gate_repo(
            r#"
            // This handler relies on isShortcutContextBlocked().
            const onKey = (e: KeyboardEvent) => {
              if (!e.metaKey) return;
              if (document.activeElement) return;
            };
            window.addEventListener("keydown", onKey);
        "#,
        );
        let out = run(check_shortcut_gate_is_shared, &d);
        assert_eq!(out.len(), 1, "a comment must not satisfy the gate: {out:?}");
    }

    /// Focus traps listen for bare Tab and MUST keep working while a
    /// modal is open — the opposite of what this gate enforces.
    #[test]
    fn shortcut_gate_ignores_non_modifier_handlers() {
        // `useFocusTrap` listens for Tab and MUST keep working while a
        // modal is open — the opposite of what this gate enforces.
        let d = gate_repo(
            r#"
            const onKey = (e: KeyboardEvent) => {
              if (e.key !== "Tab") return;
              cycle();
            };
            window.addEventListener("keydown", onKey);
        "#,
        );
        assert_eq!(run(check_shortcut_gate_is_shared, &d), Vec::<String>::new());
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
        let d = shortcut_repo(
            r#"const A = [{ keys: ["⌘", "K"], labelKey: "a", key: "k" }];"#,
            r#"const onKey = (e: KeyboardEvent) => {
                 const mod = e.metaKey || e.ctrlKey;
                 if (!mod || e.shiftKey || e.altKey) return;
                 if (e.key === "k") open();
               };"#,
        );
        assert_eq!(
            run(check_shortcut_table_has_handlers, &d),
            Vec::<String>::new()
        );
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
        // The first parser read only line-initial `key: "`, which saw 2
        // of the 9 real entries and passed a planted ⌘F.
        let d = shortcut_repo(
            r#"const A = [{ keys: ["⌘", "Q"], labelKey: "a", key: "q" }];"#,
            r#"const onKey = (e: KeyboardEvent) => {
                 const mod = e.metaKey || e.ctrlKey;
                 if (!mod || e.shiftKey || e.altKey) return;
                 if (e.key === "k") open();
               };"#,
        );
        let out = run(check_shortcut_table_has_handlers, &d);
        assert!(
            out.iter().any(|p| p.contains("⌘q")),
            "a single-line entry must still be parsed: {out:?}"
        );
    }

    /// Shift reports an uppercase `e.key`; comparing either case counts.
    #[test]
    fn shortcut_table_accepts_an_uppercase_comparison() {
        // Shift reports an uppercase `e.key`, so a ⇧ binding compares
        // the capital form.
        let d = repo();
        write(
            &d,
            "src/lib/shortcutBindings.ts",
            r#"const A = [{ keys: ["⌘", "⇧", "C"], labelKey: "a", key: "c" }];"#,
        );
        write(&d, "src/hooks/useDevToggle.ts", DEV_TOGGLE);
        write(
            &d,
            "src/sections/S.tsx",
            r#"const onKey = (e: KeyboardEvent) => {
                 const mod = e.metaKey || e.ctrlKey;
                 if (!mod || !e.shiftKey || e.altKey) return;
                 if (e.key === "C") copy();
               };"#,
        );
        assert_eq!(
            run(check_shortcut_table_has_handlers, &d),
            Vec::<String>::new()
        );
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

    /// A fallback does NOT exempt an undeclared token.
    ///
    /// This test asserted the opposite until an audit found four
    /// phantom override hooks hiding behind that exemption —
    /// `--danger-border`, `--bg-warning-soft`,
    /// `--traffic-light-center-y`, and `--bad`, which this branch's
    /// own codemod had deleted while rewriting only the bare form.
    /// Each was read with a fallback and declared nowhere, so the
    /// fallback was the only value it ever took.
    #[test]
    fn tokens_catch_an_undeclared_reference_behind_a_fallback() {
        let d = repo();
        write(&d, "src/styles/tokens.css", ":root { --fg: red; }");
        write(&d, "src/sections/S.tsx", r#"color: "var(--maybe, red)""#);
        let p = run(check_no_undefined_tokens, &d);
        assert_eq!(p.len(), 1, "{p:?}");
        assert!(p[0].contains("--maybe"));
    }

    /// A DECLARED token with a fallback is still fine — the check is
    /// about the token existing, not about fallbacks being forbidden.
    #[test]
    fn tokens_allow_a_declared_reference_with_a_fallback() {
        let d = repo();
        write(&d, "src/styles/tokens.css", ":root { --fg: red; }");
        write(&d, "src/sections/S.tsx", r#"color: "var(--fg, blue)""#);
        assert!(run(check_no_undefined_tokens, &d).is_empty());
    }

    /// A locally-declared property is a `design.md` VIOLATION, not an
    /// exemption.
    ///
    /// An earlier draft whitelisted these, to silence what looked like
    /// a false positive on `--rank-col`. The rule says tokens.css is
    /// "the one place tokens are declared", so `--rank-col` was the
    /// violation and the whitelist legitimised it — a gate quietly
    /// rewriting the rule it exists to enforce.
    #[test]
    fn tokens_catch_a_property_declared_outside_tokens_css() {
        let d = repo();
        write(&d, "src/styles/tokens.css", ":root { --fg: red; }");
        write(
            &d,
            "src/sections/S.tsx",
            r#"style={{ ["--rank-col"]: "8px", width: "var(--rank-col)" }}"#,
        );
        let p = run(check_no_undefined_tokens, &d);
        assert_eq!(p.len(), 1, "{p:?}");
        assert!(p[0].contains("--rank-col"));
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

    // ── the fixes this round, each proved by the case that motivated it ──

    #[test]
    fn strip_comments_keeps_multibyte_characters() {
        // `b[i] as char` shredded ⇧/⌥/⌘ into three Latin-1 chars, so the
        // shortcut table's modifier columns read as empty and every
        // binding looked like a bare ⌘ one.
        let out = strip_comments("keys: [\"⌘\", \"⇧\", \"L\"], // ⌥ note\n");
        assert!(out.contains('⇧'), "lost ⇧: {out:?}");
        assert!(out.contains('⌘'));
        assert!(!out.contains('⌥'), "comment survived: {out:?}");
    }

    const DEV_TOGGLE: &str = r#"
        const onDevKey = (e: KeyboardEvent) => {
          if (!e.metaKey || !e.ctrlKey || !e.altKey) return;
          if (e.key !== "l" && e.key !== "L") return;
        };
    "#;

    /// A fixture carrying the UNDOCUMENTED_BY_DESIGN handler, because
    /// that list is validated in both directions: a fixture without it
    /// legitimately reports the exemption as stale.
    fn shortcut_repo(table: &str, handler: &str) -> tempfile::TempDir {
        let d = repo();
        write(&d, "src/lib/shortcutBindings.ts", table);
        write(&d, "src/hooks/useThing.ts", handler);
        write(&d, "src/hooks/useDevToggle.ts", DEV_TOGGLE);
        d
    }

    #[test]
    fn shortcuts_accept_a_table_matching_its_handlers() {
        let d = shortcut_repo(
            r#"export const GLOBAL_SHORTCUTS = [
                 { keys: ["⌘", "⇧", "L"], labelKey: "focusLive", key: "l" },
               ];"#,
            &format!(
                r#"{DEV_TOGGLE}
                const onKey = (e: KeyboardEvent) => {{
                  const mod = e.metaKey || e.ctrlKey;
                  if (!mod || !e.shiftKey || e.altKey) return;
                  if (e.key !== "l" && e.key !== "L") return;
                }};"#
            ),
        );
        assert_eq!(
            run(check_shortcut_table_has_handlers, &d),
            Vec::<String>::new()
        );
    }

    #[test]
    fn shortcuts_catch_a_handler_answering_for_a_different_modifier() {
        // The finding: matching the bare key let ⌃⌥⌘L stand in for ⌘⇧L,
        // so deleting the ⌘⇧L handler kept this green.
        let d = shortcut_repo(
            r#"export const GLOBAL_SHORTCUTS = [
                 { keys: ["⌘", "⇧", "L"], labelKey: "focusLive", key: "l" },
               ];"#,
            DEV_TOGGLE,
        );
        let out = run(check_shortcut_table_has_handlers, &d);
        assert!(
            out.iter()
                .any(|p| p.contains("⇧⌘l") && p.contains("no handler")),
            "{out:?}"
        );
    }

    #[test]
    fn shortcuts_catch_an_undocumented_handler() {
        let d = shortcut_repo(
            r#"export const GLOBAL_SHORTCUTS = [
                 { keys: ["⌘", "K"], labelKey: "openPalette", key: "k" },
               ];"#,
            r#"const onKey = (e: KeyboardEvent) => {
                 const mod = e.metaKey || e.ctrlKey;
                 if (!mod || e.shiftKey || e.altKey) return;
                 if (e.key === "k") return;
                 if (e.key === "j") return;
               };"#,
        );
        let out = run(check_shortcut_table_has_handlers, &d);
        assert!(out.iter().any(|p| p.contains("⌘j")), "{out:?}");
    }

    #[test]
    fn shortcuts_match_the_escaped_backslash_binding() {
        // ⌘\ reads as TWO characters in source on both sides. Unescaping
        // one side only made it report missing in both directions at
        // once — the binding the whole check is named after.
        let d = shortcut_repo(
            r#"export const GLOBAL_SHORTCUTS = [
                 { keys: ["⌘", "\\"], labelKey: "toggleSidebar", key: "\\" },
               ];"#,
            r#"const onKey = (e: KeyboardEvent) => {
                 const mod = e.metaKey || e.ctrlKey;
                 if (!mod || e.shiftKey || e.altKey) return;
                 if (e.key !== "\\") return;
               };"#,
        );
        assert_eq!(
            run(check_shortcut_table_has_handlers, &d),
            Vec::<String>::new()
        );
    }

    #[test]
    fn shortcuts_reject_an_exemption_whose_handler_is_gone() {
        // Same contract as UNSUBSCRIBED_BY_DESIGN: an entry may not
        // outlive its rationale, or it reads as evidence the shortcut
        // still exists. Deliberately does NOT use `shortcut_repo`,
        // which writes the exempt handler.
        let d = repo();
        write(
            &d,
            "src/lib/shortcutBindings.ts",
            r#"const A = [{ keys: ["⌘", "K"], labelKey: "a", key: "k" }];"#,
        );
        write(
            &d,
            "src/hooks/useThing.ts",
            r#"const onKey = (e: KeyboardEvent) => {
                 const mod = e.metaKey || e.ctrlKey;
                 if (!mod || e.shiftKey || e.altKey) return;
                 if (e.key === "k") return;
               };"#,
        );
        let out = run(check_shortcut_table_has_handlers, &d);
        assert!(
            out.iter().any(|p| p.contains("UNDOCUMENTED_BY_DESIGN")),
            "{out:?}"
        );
    }

    #[test]
    fn contrast_catches_a_redeclaration_between_two_blocks() {
        // Scanning only after the LAST block let a plain re-declaration
        // sitting between them defeat the first one silently.
        let d = repo();
        write(
            &d,
            "src/styles/tokens.css",
            r#"
            :root { --focus-ring: 1px solid blue; }
            @media (prefers-contrast: more) {
              :root { --focus-ring: 3px solid black; }
            }
            :root { --focus-ring: 1px solid blue; }
            @media (prefers-contrast: more) and (prefers-color-scheme: dark) {
              :root { --line: white; }
            }
            "#,
        );
        let out = run(check_contrast_overrides_win, &d);
        assert!(
            out.iter()
                .any(|p| p.contains("--focus-ring") && p.contains("inert")),
            "{out:?}"
        );
    }

    #[test]
    fn contrast_accepts_two_blocks_that_each_come_after_their_tokens() {
        let d = repo();
        write(
            &d,
            "src/styles/tokens.css",
            r#"
            :root { --focus-ring: 1px solid blue; --line: grey; }
            @media (prefers-contrast: more) {
              :root { --focus-ring: 3px solid black; }
            }
            @media (prefers-contrast: more) and (prefers-color-scheme: dark) {
              :root { --line: white; }
            }
            "#,
        );
        assert_eq!(run(check_contrast_overrides_win, &d), Vec::<String>::new());
    }

    #[test]
    fn shortcut_gate_rejects_an_import_that_is_never_called() {
        // The check was satisfied by the symbol appearing anywhere, so
        // an unused import passed while the listener stayed ungated.
        let d = gate_repo(
            r#"
            import { isShortcutContextBlocked } from "./useGlobalShortcuts";
            const onKey = (e: KeyboardEvent) => {
              if (!e.metaKey) return;
              if (document.activeElement) return;
            };
            window.addEventListener("keydown", onKey);
        "#,
        );
        let out = run(check_shortcut_gate_is_shared, &d);
        assert_eq!(
            out.len(),
            1,
            "an unused import must not satisfy the gate: {out:?}"
        );
    }

    // ── the Codex refutation round ───────────────────────────────────

    #[test]
    fn strip_comments_does_not_treat_a_url_as_a_comment() {
        // `url(https://x)` ate the rest of the line, hiding every token
        // reference after it — a false negative in a check whose job is
        // noticing a missing token.
        let out = strip_comments(".a { background: url(https://x); color: var(--missing); }");
        assert!(out.contains("--missing"), "{out:?}");
    }

    #[test]
    fn strip_comments_does_not_treat_a_string_as_a_comment() {
        let out = strip_comments(r#"const a = "// not a comment"; const b = "--kept";"#);
        assert!(out.contains("--kept"), "{out:?}");
    }

    fn runtime_repo(tokens: &str, writer: &str, reader: &str) -> tempfile::TempDir {
        let d = repo();
        write(&d, "src/styles/tokens.css", tokens);
        write(&d, "src/lib/runtime.ts", writer);
        write(&d, "src/sections/S.tsx", reader);
        d
    }

    #[test]
    fn runtime_tokens_accept_a_declared_and_read_channel() {
        let d = runtime_repo(
            ":root { --chrome-height: 38px; --tl-center: 19px; }",
            r#"root.style.setProperty("--tl-center", "21px");"#,
            r#"const x = "calc(var(--tl-center) - 1px)";"#,
        );
        assert_eq!(
            run(check_runtime_tokens_are_registered, &d),
            Vec::<String>::new()
        );
    }

    #[test]
    fn runtime_tokens_catch_an_unregistered_channel() {
        let d = runtime_repo(
            ":root { --chrome-height: 38px; }",
            r#"root.style.setProperty("--tl-center", "21px");"#,
            r#"const x = "calc(var(--tl-center) - 1px)";"#,
        );
        let out = run(check_runtime_tokens_are_registered, &d);
        assert!(out.iter().any(|p| p.contains("never declares")), "{out:?}");
    }

    #[test]
    fn runtime_tokens_catch_a_write_whose_reader_was_deleted() {
        // The regression this check exists for: a token sweep replaced
        // the only `var(--tl-center, …)` with its fallback, on the
        // reasoning that an undeclared token could not be doing
        // anything. It was being written from AppKit at runtime.
        let d = runtime_repo(
            ":root { --chrome-height: 38px; --tl-center: 19px; }",
            r#"root.style.setProperty("--tl-center", "21px");"#,
            r#"const x = "calc(var(--chrome-height) / 2 - var(--chrome-height) / 2)";"#,
        );
        let out = run(check_runtime_tokens_are_registered, &d);
        assert!(
            out.iter().any(|p| p.contains("nothing reads it")),
            "{out:?}"
        );
    }

    #[test]
    fn contrast_rejects_a_commented_out_block() {
        let d = repo();
        write(
            &d,
            "src/styles/tokens.css",
            ":root { --line: red; }\n/*\n@media (prefers-contrast: more) {\n  :root { --line: black; }\n}\n*/\n",
        );
        let out = run(check_contrast_overrides_win, &d);
        assert!(out.iter().any(|p| p.contains("no live")), "{out:?}");
    }

    #[test]
    fn contrast_rejects_the_wrong_preference_value() {
        // `no-preference` is a different — in places opposite —
        // preference, and matching the bare prefix accepted it.
        let d = repo();
        write(
            &d,
            "src/styles/tokens.css",
            ":root { --line: red; }\n@media (prefers-contrast: no-preference) {\n  :root { --line: blue; }\n}\n",
        );
        let out = run(check_contrast_overrides_win, &d);
        assert!(out.iter().any(|p| p.contains("no live")), "{out:?}");
    }

    #[test]
    fn contrast_catches_a_declaration_split_across_lines() {
        let d = repo();
        write(
            &d,
            "src/styles/tokens.css",
            ":root { --focus-ring: blue; }\n@media (prefers-contrast: more) {\n  :root { --focus-ring: black; }\n}\n:root {\n  --focus-ring\n    : blue;\n}\n",
        );
        let out = run(check_contrast_overrides_win, &d);
        assert!(out.iter().any(|p| p.contains("inert")), "{out:?}");
    }

    #[test]
    fn contrast_accepts_a_later_block_restoring_the_override() {
        let d = repo();
        write(
            &d,
            "src/styles/tokens.css",
            "@media (prefers-contrast: more) {\n  :root { --line: black; }\n}\n:root { --line: red; }\n@media (prefers-contrast: more) {\n  :root { --line: black; }\n}\n",
        );
        assert_eq!(run(check_contrast_overrides_win, &d), Vec::<String>::new());
    }

    #[test]
    fn shortcuts_model_required_ctrl() {
        // Dropping `!e.ctrlKey` turns ⌃⌥⌘B into ⌥⌘B. With only shift
        // and alt modelled, both sides inferred the same binding and
        // the change was invisible.
        let d = shortcut_repo(
            r#"const A = [{ keys: ["⌃", "⌥", "⌘", "B"], labelKey: "b", key: "b" }];"#,
            r#"const onBoardsKey = (e: KeyboardEvent) => {
                 if (!e.metaKey || !e.altKey) return;
                 if (e.key !== "b") return;
               };"#,
        );
        let out = run(check_shortcut_table_has_handlers, &d);
        assert!(out.iter().any(|p| p.contains("⌃⌥⌘b")), "{out:?}");
    }

    #[test]
    fn shortcuts_catch_a_chip_that_disagrees_with_the_handler_key() {
        // `keys` renders the chips, `key` is what the handler compares.
        // They are separate fields, so changing one advertises a
        // shortcut nobody wired.
        let d = shortcut_repo(
            r#"const A = [{ keys: ["⌘", "Q"], labelKey: "a", key: "k" }];"#,
            r#"const onKey = (e: KeyboardEvent) => {
                 const mod = e.metaKey || e.ctrlKey;
                 if (!mod || e.shiftKey || e.altKey) return;
                 if (e.key === "k") open();
               };"#,
        );
        let out = run(check_shortcut_table_has_handlers, &d);
        assert!(out.iter().any(|p| p.contains("final chip")), "{out:?}");
    }

    #[test]
    fn shortcuts_parse_an_entry_whose_key_precedes_keys() {
        // Splitting on `keys: [` assumed a property order TypeScript
        // does not require, so a row written the other way was skipped
        // entirely — and a dead binding written that way passed.
        let d = shortcut_repo(
            r#"const A = [{ key: "q", labelKey: "a", keys: ["⌘", "Q"] }];"#,
            r#"const onKey = (e: KeyboardEvent) => {
                 const mod = e.metaKey || e.ctrlKey;
                 if (!mod || e.shiftKey || e.altKey) return;
                 if (e.key === "k") open();
               };"#,
        );
        let out = run(check_shortcut_table_has_handlers, &d);
        assert!(out.iter().any(|p| p.contains("⌘q")), "{out:?}");
    }

    #[test]
    fn shortcuts_ignore_a_handler_that_rejects_modifiers() {
        // `if (e.metaKey || e.ctrlKey) return;` is the OPPOSITE of a
        // shortcut, and requiring only that the scope mention metaKey
        // let it answer for a documented binding.
        let d = repo();
        write(
            &d,
            "src/lib/shortcutBindings.ts",
            r#"const A = [{ keys: ["⌘", "J"], labelKey: "a", key: "j" }];"#,
        );
        write(&d, "src/hooks/useDevToggle.ts", DEV_TOGGLE);
        write(
            &d,
            "src/sections/List.tsx",
            r#"const onListKey = (e: KeyboardEvent) => {
                 if (e.metaKey || e.ctrlKey) return;
                 if (e.key === "j") moveCursor();
               };"#,
        );
        let out = run(check_shortcut_table_has_handlers, &d);
        assert!(
            out.iter()
                .any(|p| p.contains("⌘j") && p.contains("no handler")),
            "a cursor handler must not answer for ⌘J: {out:?}"
        );
    }

    #[test]
    fn shortcuts_report_a_modifier_form_they_cannot_read() {
        // Guessing produced confident wrong answers, so an unreadable
        // guard is REPORTED rather than assigned a modifier set.
        let d = shortcut_repo(
            r#"const A = [{ keys: ["⌘", "K"], labelKey: "a", key: "k" }];"#,
            r#"const onKey = (e: KeyboardEvent) => {
                 const mod = e.metaKey || e.ctrlKey;
                 if (!mod || e.shiftKey === false) return;
                 if (e.key === "k") open();
               };"#,
        );
        let out = run(check_shortcut_table_has_handlers, &d);
        assert!(out.iter().any(|p| p.contains("cannot read")), "{out:?}");
    }

    #[test]
    fn shortcuts_see_a_switch_case_binding() {
        let d = shortcut_repo(
            r#"const A = [{ keys: ["⌘", "K"], labelKey: "a", key: "k" }];"#,
            r#"const onKey = (e: KeyboardEvent) => {
                 const mod = e.metaKey || e.ctrlKey;
                 if (!mod || e.shiftKey || e.altKey) return;
                 switch (e.key) {
                   case "k": open(); break;
                 }
               };"#,
        );
        assert_eq!(
            run(check_shortcut_table_has_handlers, &d),
            Vec::<String>::new()
        );
    }

    #[test]
    fn gate_is_checked_per_handler_not_per_file() {
        // One gated handler used to license every other listener in the
        // same file; `useShellShortcuts` registers four.
        let d = gate_repo(
            r#"
            import { isShortcutContextBlocked } from "./useGlobalShortcuts";
            const onKey = (e: KeyboardEvent) => {
              if (!e.metaKey) return;
              if (isShortcutContextBlocked()) return;
            };
            const onOther = (e: KeyboardEvent) => {
              if (!e.metaKey) return;
              act();
            };
        "#,
        );
        let out = run(check_shortcut_gate_is_shared, &d);
        assert!(out.iter().any(|p| p.contains("onOther")), "{out:?}");
    }

    #[test]
    fn gate_is_not_satisfied_by_a_string_containing_the_call() {
        let d = gate_repo(
            r#"
            const onKey = (e: KeyboardEvent) => {
              if (!e.metaKey) return;
              const d = "isShortcutContextBlocked()";
              act();
            };
        "#,
        );
        let out = run(check_shortcut_gate_is_shared, &d);
        assert_eq!(out.len(), 1, "{out:?}");
    }

    #[test]
    fn callbacks_catch_a_shortcut_nobody_can_trigger() {
        // The ⌘F bug in its original form: documented, wired, and inert
        // because no caller ever passed the callback it gates on.
        let d = repo();
        write(
            &d,
            "src/hooks/useGlobalShortcuts.ts",
            r#"export function useGlobalShortcuts(opts) {
                 const onKey = (e: KeyboardEvent) => {
                   if (!e.metaKey) return;
                   if (e.key === "n" && onAdd) onAdd();
                 };
               }"#,
        );
        write(
            &d,
            "src/sections/S.tsx",
            "useGlobalShortcuts({ onRefresh: r });",
        );
        let out = run(check_optional_shortcut_callbacks_are_wired, &d);
        assert!(out.iter().any(|p| p.contains("onAdd")), "{out:?}");
    }

    #[test]
    fn callbacks_accept_one_that_a_call_site_passes() {
        let d = repo();
        write(
            &d,
            "src/hooks/useGlobalShortcuts.ts",
            r#"export function useGlobalShortcuts(opts) {
                 const onKey = (e: KeyboardEvent) => {
                   if (!e.metaKey) return;
                   if (e.key === "n" && onAdd) onAdd();
                 };
               }"#,
        );
        write(
            &d,
            "src/sections/S.tsx",
            "useGlobalShortcuts({ onAdd: add });",
        );
        assert_eq!(
            run(check_optional_shortcut_callbacks_are_wired, &d),
            Vec::<String>::new()
        );
    }

    #[test]
    fn declaration_site_accepts_tokens_css_alone() {
        let d = repo();
        write(&d, "src/styles/tokens.css", ":root { --fg: red; }");
        write(
            &d,
            "src/styles/components/base.css",
            ".x { color: var(--fg); }",
        );
        assert_eq!(
            run(check_tokens_declared_only_in_tokens_css, &d),
            Vec::<String>::new()
        );
    }

    #[test]
    fn declaration_site_catches_a_stray_stylesheet_declaration() {
        let d = repo();
        write(&d, "src/styles/tokens.css", ":root { --fg: red; }");
        write(&d, "src/styles/components/base.css", ".x { --stray: 3px; }");
        let out = run(check_tokens_declared_only_in_tokens_css, &d);
        assert!(out.iter().any(|p| p.contains("--stray")), "{out:?}");
    }

    #[test]
    fn declaration_site_catches_an_inline_style_declaration() {
        // The shape that earned the token check its "locally declared"
        // exemption: `--rank-col` declared on a grid container and read
        // two lines below.
        let d = repo();
        write(&d, "src/styles/tokens.css", ":root { --fg: red; }");
        write(
            &d,
            "src/sections/S.tsx",
            r#"const a = { ["--rank-col" as keyof React.CSSProperties]: "28px" };"#,
        );
        let out = run(check_tokens_declared_only_in_tokens_css, &d);
        assert!(out.iter().any(|p| p.contains("--rank-col")), "{out:?}");
    }

    #[test]
    fn declaration_site_allows_a_runtime_setproperty() {
        // The runtime channel is a different mechanism with its own
        // check — it must NAME a declared token, the opposite of
        // declaring one elsewhere.
        let d = repo();
        write(&d, "src/styles/tokens.css", ":root { --fg: red; }");
        write(
            &d,
            "src/lib/runtime.ts",
            r#"root.style.setProperty("--fg", "blue");"#,
        );
        assert_eq!(
            run(check_tokens_declared_only_in_tokens_css, &d),
            Vec::<String>::new()
        );
    }
}
