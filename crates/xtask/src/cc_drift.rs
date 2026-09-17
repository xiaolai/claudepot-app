//! `cargo xtask cc-drift` — has Claude Code moved under us?
//!
//! Claude Code ships ~27 releases a month. Claudepot reimplements,
//! parses, or depends on CC behaviour in ~20 places, and nothing used to
//! notice when one of those releases changed one of them —
//! `cleanupPeriodDays` sat inverted for some unknown part of 145
//! releases before anyone looked.
//!
//! # The split, which is the whole design
//!
//! **Measurement needs a machine; judgement does not.** Reading the
//! installed CC binary needs CC installed; fetching the changelog needs
//! the network. Deciding whether a diff touches something we own is pure
//! text work. So:
//!
//! - the pure half ([`parse_watchlist`], [`scan_changelog`],
//!   [`compare_pins`]) is unit-tested in CI against fixtures;
//! - the measuring half ([`run`]) is on demand only.
//!
//! This is the `check:envvar-layout` lesson applied before it bites: a
//! guard whose judgement can only run where the measurement runs is a
//! guard nobody has watched fail.
//!
//! # Not a CI gate
//!
//! CI has no CC installed and the version moves daily, so gating on
//! drift would be permanently red — and `AGENTS.md` already records
//! where a permanently-red gate leads (`--no-verify` as reflex). What
//! CI *does* enforce is that the watchlist itself stays well-formed and
//! its owners exist; see `verify_docs::check_cc_watchlist`.

use anyhow::{bail, Context, Result};
use std::path::Path;

/// Path of the watchlist this tool reads, relative to the workspace
/// root.
///
/// It lives beside the tool rather than in `.claude/rules/` because
/// everything in that directory is loaded into every Claude Code session
/// in this repo. A monthly routine's 21-row target list does not earn
/// always-on context; the short rule that points here does, and stays
/// there.
pub const WATCHLIST_REL: &str = "crates/xtask/cc-upstream-watch.md";

/// One row of the watchlist table: a CC surface we depend on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchRow {
    pub surface: String,
    pub owner: String,
    /// Literal substrings to look for. A trailing `*` is a prefix match
    /// (`CLAUDE_*`), because that is how the interesting env-var
    /// families are written. Empty when the row's check is not
    /// text-searchable (`claude doctor` output, for instance) — those
    /// rows carry `—` and are skipped by the scan rather than silently
    /// matching everything.
    pub tokens: Vec<String>,
    pub check: String,
    /// The newest Claude Code version this row records having been checked
    /// against (`Verified 2.1.250`, `Re-read in 2.1.274`, …), or `None`
    /// when it records none. It is the row's own baseline: only releases
    /// after it are candidates for this row.
    pub verified: Option<String>,
    /// Set when the row watches another binary — `Verified Desktop
    /// 2.110.0` — whose releases are not in CC's changelog. Such a row
    /// is never scanned against it: every CC release would count as a
    /// candidate forever, and a check that always fires is one people
    /// learn to skip.
    pub other_binary: Option<String>,
}

/// A watchlist token found in an upstream release note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hit {
    pub version: String,
    pub token: String,
    pub surface: String,
    pub owner: String,
    pub check: String,
    /// The bullet it appeared in, so the report is readable without
    /// opening the changelog.
    pub excerpt: String,
}

/// Parse the watchlist table out of the committed input file.
///
/// Deliberately strict: a table this file cannot read is a table the
/// drift check silently skips, which is the failure mode the whole
/// routine exists to avoid.
pub fn parse_watchlist(md: &str) -> Result<Vec<WatchRow>> {
    let mut rows = Vec::new();
    let mut in_table = false;
    for line in md.lines() {
        let t = line.trim();
        if t.starts_with("## ") {
            // Only the table under "## The watchlist" counts; the file
            // has other tables (signals, version pins) that are not
            // watch rows.
            in_table = t == "## The watchlist";
            continue;
        }
        if !in_table || !t.starts_with('|') {
            continue;
        }
        let cells: Vec<&str> = t.trim_matches('|').split('|').map(str::trim).collect();
        if cells.len() != 4 {
            bail!("watchlist row has {} cells, expected 4: {t}", cells.len());
        }
        // Header and the |---| separator.
        if cells[0] == "CC surface" || cells[0].starts_with("---") {
            continue;
        }
        rows.push(WatchRow {
            surface: cells[0].to_string(),
            owner: cells[1].to_string(),
            tokens: parse_tokens(cells[2]),
            check: cells[3].to_string(),
            verified: parse_verified(cells[3]),
            other_binary: parse_other_binary(cells[3]),
        });
    }
    if rows.is_empty() {
        bail!("no watchlist rows parsed from {WATCHLIST_REL} — the table format changed");
    }
    Ok(rows)
}

/// The newest version a check cell says it was verified against.
///
/// Rows record it in prose — `**Verified 2.1.250 (2026-08-28).**`,
/// `**Re-read in 2.1.274 (…)**`, `Reconciled 2.1.250`, `Ran 2.1.274` — so
/// this accepts those verbs followed by a version and keeps the highest.
fn parse_verified(check: &str) -> Option<String> {
    const VERBS: [&str; 7] = [
        "verified against ",
        "verified ",
        "re-verified ",
        "re-read in ",
        "reconciled ",
        "read ",
        "ran ",
    ];
    let lower = check.to_ascii_lowercase();
    let mut best: Option<String> = None;
    for verb in VERBS {
        let mut from = 0;
        while let Some(at) = lower[from..].find(verb) {
            let start = from + at + verb.len();
            let version: String = lower[start..]
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            let version = version.trim_end_matches('.');
            if version.split('.').count() == 3 && version.split('.').all(|p| !p.is_empty()) {
                let newer = best.as_deref().is_none_or(|b| is_newer(version, b));
                if newer {
                    best = Some(version.to_string());
                }
            }
            from = start;
        }
    }
    best
}

/// `Verified Desktop X.Y.Z` → `Desktop X.Y.Z`. The newest such marker
/// wins, by the same version ordering as [`parse_verified`].
fn parse_other_binary(check: &str) -> Option<String> {
    const MARKER: &str = "verified desktop ";
    let lower = check.to_ascii_lowercase();
    let mut best: Option<String> = None;
    let mut from = 0;
    while let Some(at) = lower[from..].find(MARKER) {
        let start = from + at + MARKER.len();
        let version: String = lower[start..]
            .chars()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        let version = version.trim_end_matches('.').to_string();
        if version.split('.').count() >= 2 && version.split('.').all(|p| !p.is_empty()) {
            let newer = best.as_deref().is_none_or(|b| is_newer(&version, b));
            if newer {
                best = Some(version);
            }
        }
        from = start;
    }
    best.map(|v| format!("Desktop {v}"))
}

/// Split a tokens cell into literal needles. `—` means "no text search
/// applies to this row" and yields none.
fn parse_tokens(cell: &str) -> Vec<String> {
    if cell == "—" || cell == "-" || cell.is_empty() {
        return Vec::new();
    }
    cell.split(',')
        .map(|t| t.trim().trim_matches('`').trim().to_string())
        .filter(|t| !t.is_empty())
        .collect()
}

/// Does `haystack` contain `token`, honouring a trailing `*` as a
/// prefix family match?
fn token_matches(token: &str, haystack: &str) -> bool {
    match token.strip_suffix('*') {
        // `CLAUDE_*` must not match the bare word "CLAUDE_" in prose;
        // require at least one more name character after the prefix.
        Some(prefix) => haystack.match_indices(prefix).any(|(i, _)| {
            haystack[i + prefix.len()..]
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
        }),
        None => haystack.contains(token),
    }
}

/// One upstream release: its version and its bullets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    pub bullets: Vec<String>,
}

/// Split `CHANGELOG.md` into releases, newest first.
pub fn parse_changelog(md: &str) -> Vec<Release> {
    let mut out: Vec<Release> = Vec::new();
    for line in md.lines() {
        let t = line.trim_end();
        if let Some(v) = t.strip_prefix("## ") {
            out.push(Release {
                version: v.trim().to_string(),
                bullets: Vec::new(),
            });
        } else if let Some(b) = t.trim_start().strip_prefix("- ") {
            if let Some(cur) = out.last_mut() {
                cur.bullets.push(b.trim().to_string());
            }
        }
    }
    out
}

/// Compare two dotted numeric versions. Non-numeric segments sort last
/// so a pre-release tag never reads as newer than a release.
fn version_key(v: &str) -> Vec<u64> {
    v.split('.')
        .map(|p| p.trim().parse::<u64>().unwrap_or(0))
        .collect()
}

/// Is `a` strictly newer than `b`?
pub fn is_newer(a: &str, b: &str) -> bool {
    version_key(a) > version_key(b)
}

/// Find every watchlist token mentioned in releases newer than `since`.
///
/// Pure: the caller supplies the changelog text.
pub fn scan_changelog(rows: &[WatchRow], changelog: &str, since: &str) -> Vec<Hit> {
    scan_with(rows, changelog, |_| since.to_string())
}

/// Each row since its own recorded verification.
///
/// One shared baseline cannot be right for rows verified at different
/// versions. It used to be the parity pin, which was safe only while that
/// pin was the oldest version anywhere (2.1.88): the day it moved to the
/// installed build, every row verified earlier had its later changelog
/// entries hidden and the report printed green. A row that records no
/// verification reports everything.
pub fn scan_changelog_per_row(rows: &[WatchRow], changelog: &str) -> Vec<Hit> {
    scan_with(rows, changelog, |row| {
        row.verified.clone().unwrap_or_else(|| "0.0.0".to_string())
    })
}

fn scan_with(
    rows: &[WatchRow],
    changelog: &str,
    baseline: impl Fn(&WatchRow) -> String,
) -> Vec<Hit> {
    let mut hits = Vec::new();
    for rel in parse_changelog(changelog) {
        for bullet in &rel.bullets {
            for row in rows {
                if row.other_binary.is_some() || !is_newer(&rel.version, &baseline(row)) {
                    continue;
                }
                for token in &row.tokens {
                    if token_matches(token, bullet) {
                        hits.push(Hit {
                            version: rel.version.clone(),
                            token: token.clone(),
                            surface: row.surface.clone(),
                            owner: row.owner.clone(),
                            check: row.check.clone(),
                            excerpt: bullet.clone(),
                        });
                    }
                }
            }
        }
    }
    hits
}

/// Hits for one watchlist surface, collapsed.
///
/// The raw list is unusable as a report: over 115 releases the real
/// watchlist produces ~200 hits, 128 of them the env-var family, and a
/// reader who scrolls 200 near-identical stanzas stops reading. What is
/// actionable is one line per surface plus the command that settles it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SurfaceReport {
    pub surface: String,
    pub owner: String,
    pub check: String,
    pub mentions: usize,
    /// Newest release mentioning this surface.
    pub newest: String,
    /// Up to [`EXAMPLES_PER_SURFACE`] bullets, newest first.
    pub examples: Vec<(String, String)>,
}

/// How many example bullets each surface shows. The remainder is always
/// printed as "+N more" — a silent cap reads as "that was everything".
pub const EXAMPLES_PER_SURFACE: usize = 3;

/// Collapse hits to one entry per surface, newest surface first. Pure.
pub fn group_hits(hits: &[Hit]) -> Vec<SurfaceReport> {
    let mut out: Vec<SurfaceReport> = Vec::new();
    for h in hits {
        match out.iter_mut().find(|r| r.surface == h.surface) {
            Some(r) => {
                r.mentions += 1;
                if is_newer(&h.version, &r.newest) {
                    r.newest = h.version.clone();
                }
                // Dedupe by bullet: one bullet naming three env vars is
                // one fact, not three.
                if r.examples.len() < EXAMPLES_PER_SURFACE
                    && !r.examples.iter().any(|(_, e)| e == &h.excerpt)
                {
                    r.examples.push((h.version.clone(), h.excerpt.clone()));
                }
            }
            None => out.push(SurfaceReport {
                surface: h.surface.clone(),
                owner: h.owner.clone(),
                check: h.check.clone(),
                mentions: 1,
                newest: h.version.clone(),
                examples: vec![(h.version.clone(), h.excerpt.clone())],
            }),
        }
    }
    out.sort_by(|a, b| {
        version_key(&b.newest)
            .cmp(&version_key(&a.newest))
            .then(b.mentions.cmp(&a.mentions))
    });
    out
}

/// What shape a pin's value has, and therefore what "stale" means.
///
/// Comparing every pin to the installed CC version treated
/// `docs_fetched_at` (a date) as a version, so those rows reported
/// "BEHIND 2.1.233" on every run forever — noise by construction, and
/// noise is what teaches a reader to skim the report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinKind {
    /// A CC version string, comparable to the installed one.
    Version,
    /// A date or hash. Freshness evidence — reported, never compared to
    /// a version.
    Freshness,
}

/// A version-pinned artifact and what it claims.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pin {
    pub artifact: String,
    pub field: String,
    pub value: String,
    pub kind: PinKind,
    /// What silently stops working while this is stale. The point of
    /// the report: every one of these disables itself and says nothing.
    pub when_stale: String,
}

/// A pin that no longer matches the installed CC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinFinding {
    pub pin: Pin,
    pub installed: String,
}

/// Which **version** pins are behind the installed CC. Pure.
///
/// Freshness pins are deliberately excluded: a date can never equal a
/// version, so including them guaranteed a permanent "BEHIND" row that
/// carried no information.
pub fn compare_pins(installed: &str, pins: &[Pin]) -> Vec<PinFinding> {
    pins.iter()
        .filter(|p| p.kind == PinKind::Version && p.value != installed)
        .map(|p| PinFinding {
            pin: (*p).clone(),
            installed: installed.to_string(),
        })
        .collect()
}

// ─────────────────────────── measurement ───────────────────────────

/// Newest version directory under `~/.local/share/claude/versions/`,
/// falling back to `claude --version`.
fn installed_cc_version() -> Result<String> {
    if let Some(home) = std::env::var_os("HOME") {
        let dir = Path::new(&home)
            .join(".local")
            .join("share")
            .join("claude")
            .join("versions");
        if let Ok(rd) = std::fs::read_dir(&dir) {
            let mut versions: Vec<String> = rd
                .filter_map(|e| e.ok())
                .filter_map(|e| e.file_name().into_string().ok())
                .filter(|n| n.chars().next().is_some_and(|c| c.is_ascii_digit()))
                .collect();
            versions.sort_by_key(|v| version_key(v));
            if let Some(v) = versions.pop() {
                return Ok(v);
            }
        }
    }
    let out = std::process::Command::new("claude")
        .arg("--version")
        .output()
        .context("neither ~/.local/share/claude/versions nor `claude --version` is available")?;
    let s = String::from_utf8_lossy(&out.stdout);
    s.split_whitespace()
        .next()
        .map(str::to_string)
        .context("could not parse `claude --version` output")
}

/// Read the pins this repo carries, plus any artifact that could not be
/// read.
///
/// The second return value is load-bearing. Skipping an unreadable pin
/// silently let `run()` fall back to `since = installed`, which makes
/// every release "not newer" and prints "0 releases newer" — a missing
/// file rendering as a clean run. Absence must be reported, never
/// absorbed.
fn read_pins(root: &Path) -> Result<(Vec<Pin>, Vec<String>)> {
    let mut pins = Vec::new();
    let mut problems = Vec::new();

    let parity = root.join("parity-harness/PINNED_CC_VERSION");
    match std::fs::read_to_string(&parity) {
        Ok(v) => pins.push(Pin {
            artifact: "parity-harness/PINNED_CC_VERSION".into(),
            field: "(file)".into(),
            value: v.trim().to_string(),
            kind: PinKind::Version,
            when_stale: "settings-merge fixtures were verified against an older build — \
                         re-run `bun parity-harness/dump.ts --check`"
                .into(),
        }),
        Err(e) => problems.push(format!(
            "parity-harness/PINNED_CC_VERSION could not be read ({e}) — the \
             settings-merge fixtures have no recorded verification"
        )),
    }

    let ev_path = root.join("crates/claudepot-core/data/cc-env-evidence.json");
    let ev_text = std::fs::read_to_string(&ev_path);
    if ev_text.is_err() {
        problems.push(
            "cc-env-evidence.json could not be read — the env pane's build crosscheck \
             state is unknown, not current"
                .to_string(),
        );
    }
    if let Ok(text) = ev_text {
        let ev: serde_json::Value =
            serde_json::from_str(&text).with_context(|| format!("parse {}", ev_path.display()))?;
        for (field, kind, when_stale) in [
            (
                "binary_crosscheck_version",
                PinKind::Version,
                "env pane renders present_in_build / undocumented_in_build as \"unavailable for \
                 this version\" — correct, but it is a state nobody can act on",
            ),
            (
                "cc_source_version",
                PinKind::Version,
                "env pane's pre-trust / provider-managed flags describe an older build",
            ),
            (
                "cc_source_read_at",
                PinKind::Freshness,
                "when the safety lists were read from that build",
            ),
            (
                "docs_fetched_at",
                PinKind::Freshness,
                "documented rows drift from the live docs page",
            ),
        ] {
            if let Some(v) = ev.get(field).and_then(|v| v.as_str()) {
                pins.push(Pin {
                    artifact: "cc-env-evidence.json".into(),
                    field: field.into(),
                    value: v.to_string(),
                    kind,
                    when_stale: when_stale.into(),
                });
            }
        }
    }
    Ok((pins, problems))
}

/// Fetch the upstream changelog with `gh`, or read `--changelog <path>`.
fn load_changelog(explicit: Option<&str>) -> Result<String> {
    if let Some(p) = explicit {
        return std::fs::read_to_string(p).with_context(|| format!("read changelog {p}"));
    }
    let out = std::process::Command::new("gh")
        .args([
            "api",
            "repos/anthropics/claude-code/contents/CHANGELOG.md",
            "--jq",
            ".content",
        ])
        .output()
        .context("`gh` not available — pass --changelog <path> instead")?;
    if !out.status.success() {
        bail!(
            "gh api failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let b64: String = String::from_utf8_lossy(&out.stdout)
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    let bytes = base64_decode(&b64).context("decode changelog base64 from gh")?;
    String::from_utf8(bytes).context("changelog was not utf-8")
}

/// Minimal base64 decoder. A dev tool that runs a handful of times a
/// month does not justify a dependency, and `gh` has no raw-content
/// `--jq` path that avoids the encoding.
fn base64_decode(s: &str) -> Result<Vec<u8>> {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    for c in s.bytes() {
        if c == b'=' {
            break;
        }
        let Some(v) = T.iter().position(|&t| t == c) else {
            bail!("invalid base64 byte {c:?}");
        };
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Ok(out)
}

/// Render the artifacts we could not read. Printed first and
/// unconditionally: everything after it is only as trustworthy as the
/// inputs, and a skipped input used to be invisible.
fn render_problems(problems: &[String]) {
    if problems.is_empty() {
        return;
    }
    println!("PIN ARTIFACTS UNREADABLE — this report is incomplete:");
    for p in problems {
        println!("  ! {p}");
    }
    println!();
}

/// Render freshness evidence and version pins. Kept apart because a
/// date is not comparable to a version; conflating them printed a
/// permanent, meaningless "BEHIND" row.
fn render_pins(installed: &str, pins: &[Pin]) {
    let fresh: Vec<&Pin> = pins
        .iter()
        .filter(|p| p.kind == PinKind::Freshness)
        .collect();
    if !fresh.is_empty() {
        println!("freshness evidence (not version-comparable):");
        for p in &fresh {
            println!("  {} · {} = {}", p.artifact, p.field, p.value);
            println!("      → {}", p.when_stale);
        }
        println!();
    }

    let stale = compare_pins(installed, pins);
    if stale.is_empty() {
        println!("version pins: all current\n");
    } else {
        println!("version pins BEHIND {installed}:");
        for f in &stale {
            println!(
                "  {} · {} = {}\n      → {}",
                f.pin.artifact, f.pin.field, f.pin.value, f.pin.when_stale
            );
        }
        println!();
    }
}

/// Render the changelog scan, collapsed by surface.
fn render_changelog(rows: &[WatchRow], text: &str, since: Option<&str>) {
    let hits = match since {
        Some(since) => {
            let newer = parse_changelog(text)
                .iter()
                .filter(|r| is_newer(&r.version, since))
                .count();
            println!("changelog: {newer} releases newer than {since} (explicit --since)");
            scan_changelog(rows, text, since)
        }
        None => {
            println!("changelog: each row since its own recorded verification");
            let never: Vec<&str> = rows
                .iter()
                .filter(|r| {
                    r.verified.is_none() && r.other_binary.is_none() && !r.tokens.is_empty()
                })
                .map(|r| r.surface.as_str())
                .collect();
            if !never.is_empty() {
                println!(
                    "  never verified (every release counts): {}",
                    never.join("; ")
                );
            }
            for r in rows {
                if let Some(other) = &r.other_binary {
                    println!(
                        "  not in this changelog (check by hand): {} — last verified {other}",
                        r.surface
                    );
                }
            }
            scan_changelog_per_row(rows, text)
        }
    };
    let grouped = group_hits(&hits);
    if grouped.is_empty() {
        println!("  no watchlist token mentioned — checked, green\n");
        return;
    }
    println!(
        "  {} surfaces mentioned across {} bullets:\n",
        grouped.len(),
        hits.len()
    );
    for r in &grouped {
        println!(
            "  {} — {} mention{}, newest {}",
            r.surface,
            r.mentions,
            if r.mentions == 1 { "" } else { "s" },
            r.newest
        );
        println!("      owner: {}", r.owner);
        println!("      check: {}", r.check);
        for (v, e) in &r.examples {
            println!("      [{v}] {e}");
        }
        // Never let a cap read as completeness.
        if r.mentions > r.examples.len() {
            println!("      (+{} more)", r.mentions - r.examples.len());
        }
        println!();
    }
}

/// Entry point for `cargo xtask cc-drift`.
///
/// Measurement and orchestration only — every judgement it prints comes
/// from a pure function above, so the interesting logic is testable
/// without a Claude Code install.
pub fn run(root: &Path, args: &[String]) -> Result<()> {
    let arg = |name: &str| -> Option<&str> {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .map(String::as_str)
    };

    let watchlist_path = root.join(WATCHLIST_REL);
    let md = std::fs::read_to_string(&watchlist_path)
        .with_context(|| format!("read {}", watchlist_path.display()))?;
    let rows = parse_watchlist(&md)?;

    let installed = installed_cc_version()?;
    let (pins, pin_problems) = read_pins(root)?;

    // `--since` overrides; otherwise every row is read from its own
    // recorded verification. The installed version is never a baseline:
    // `is_newer(rel, installed)` is false for every release, so the scan
    // would print nothing and read as a clean run.
    let since = arg("--since").map(str::to_string);

    println!("cc-drift: installed Claude Code {installed}");
    println!("          {} watchlist rows\n", rows.len());

    render_problems(&pin_problems);
    render_pins(&installed, &pins);

    match load_changelog(arg("--changelog")) {
        Ok(text) => render_changelog(&rows, &text, since.as_deref()),
        Err(e) => {
            // A failed fetch must not read as "nothing changed".
            println!("changelog: NOT CHECKED — {e}");
            println!("  re-run with --changelog <path> before trusting this report\n");
        }
    }

    println!(
        "This is a report, not a gate. Record the outcome — including a \
         green one — per {WATCHLIST_REL}."
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_row_records_its_newest_verification() {
        assert_eq!(
            parse_verified("binary strings. **Verified 2.1.250 (2026-08-28).** later **Re-read in 2.1.274 (x)**"),
            Some("2.1.274".to_string())
        );
        assert_eq!(
            parse_verified("Reconciled 2.1.250 and it moved"),
            Some("2.1.250".into())
        );
        assert_eq!(
            parse_verified("**Ran 2.1.274 (2026-09-17): 8 of 8**"),
            Some("2.1.274".into())
        );
        assert_eq!(parse_verified("binary strings"), None);
        // "read" inside other words must not invent a version.
        assert_eq!(parse_verified("re-read the list; spread 2 items"), None);
    }

    #[test]
    fn a_row_about_another_binary_is_never_scanned_against_this_changelog() {
        assert_eq!(
            parse_other_binary("**Verified Desktop 1.34493.1 (…)** then Verified Desktop 2.110.0"),
            Some("Desktop 2.110.0".to_string())
        );
        assert_eq!(parse_other_binary("Verified 2.1.274"), None);
        let desktop = WatchRow {
            surface: "desktop".into(),
            owner: "o".into(),
            tokens: vec!["tokenCache".into()],
            check: "c".into(),
            verified: None,
            other_binary: Some("Desktop 2.110.0".into()),
        };
        let log = "## 2.1.270\n\n- tokenCache moved\n";
        assert!(scan_changelog_per_row(std::slice::from_ref(&desktop), log).is_empty());
        assert!(scan_changelog(&[desktop], log, "0.0.0").is_empty());
    }

    #[test]
    fn each_row_is_scanned_from_its_own_verification() {
        // The false green this replaced: one baseline newer than a row's
        // own verification hid that row's later changes.
        let row = |surface: &str, token: &str, verified: Option<&str>| WatchRow {
            surface: surface.into(),
            owner: "o".into(),
            tokens: vec![token.into()],
            check: "c".into(),
            verified: verified.map(str::to_string),
            other_binary: None,
        };
        let rows = vec![
            row("old", "alpha", Some("2.1.240")),
            row("new", "beta", Some("2.1.270")),
            row("never", "gamma", None),
        ];
        let log = "## 2.1.260\n\n- alpha and beta and gamma\n\n## 2.1.100\n\n- gamma\n";
        let surfaces: Vec<(String, String)> = scan_changelog_per_row(&rows, log)
            .into_iter()
            .map(|h| (h.surface, h.version))
            .collect();
        assert!(surfaces.contains(&("old".into(), "2.1.260".into())));
        assert!(!surfaces.iter().any(|(s, _)| s == "new"), "{surfaces:?}");
        assert!(surfaces.contains(&("never".into(), "2.1.100".into())));
        assert!(surfaces.contains(&("never".into(), "2.1.260".into())));
    }

    const TABLE: &str = "\
# heading

## Signals

| a | b |
|---|---|
| not | a watch row |

## The watchlist

| CC surface | Claudepot owner | Grep tokens | Check |
|---|---|---|---|
| `cleanupPeriodDays` | `cc_retention` | `cleanupPeriodDays`, `persistSession` | binary strings |
| env vars | `cc_env` | `CLAUDE_*`, `ANTHROPIC_*` | rebuild evidence |
| doctor output | `cc_doctor` | — | run it |

## After
";

    #[test]
    fn parses_only_the_watchlist_table() {
        let rows = parse_watchlist(TABLE).unwrap();
        assert_eq!(rows.len(), 3, "the Signals table must not be picked up");
        assert_eq!(rows[0].owner, "`cc_retention`");
        assert_eq!(rows[0].tokens, vec!["cleanupPeriodDays", "persistSession"]);
    }

    /// A row with no searchable token must yield none rather than an
    /// empty-string needle, which would match every bullet ever written.
    #[test]
    fn an_em_dash_token_cell_yields_no_needles() {
        let rows = parse_watchlist(TABLE).unwrap();
        assert!(rows[2].tokens.is_empty());
        let hits = scan_changelog(&rows[2..], "## 9.9.9\n\n- anything at all\n", "0.0.0");
        assert!(hits.is_empty(), "{hits:?}");
    }

    /// The format changing must be loud. A silently-empty watchlist
    /// makes every future run report "green".
    #[test]
    fn an_unreadable_table_is_an_error_not_an_empty_list() {
        assert!(parse_watchlist("# nothing here").is_err());
        assert!(parse_watchlist("## The watchlist\n\n| a | b |\n|---|---|\n| x | y |\n").is_err());
    }

    #[test]
    fn prefix_tokens_match_a_family_but_not_the_bare_prefix() {
        assert!(token_matches(
            "CLAUDE_*",
            "added CLAUDE_CODE_TOOL_MEMORY_LIMIT today"
        ));
        assert!(!token_matches("CLAUDE_*", "the CLAUDE_ prefix is reserved"));
        assert!(!token_matches("CLAUDE_*", "nothing here"));
    }

    #[test]
    fn only_releases_newer_than_since_are_scanned() {
        let rows = parse_watchlist(TABLE).unwrap();
        let log = "\
## 2.1.100

- Changed `cleanupPeriodDays` handling

## 2.1.88

- Changed `cleanupPeriodDays` handling
";
        let hits = scan_changelog(&rows, log, "2.1.88");
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].version, "2.1.100");
    }

    /// The bug this ordering prevents: string comparison puts "2.1.9"
    /// after "2.1.100", so a naive scan would skip 90 releases.
    #[test]
    fn version_ordering_is_numeric_not_lexicographic() {
        assert!(is_newer("2.1.100", "2.1.99"));
        assert!(is_newer("2.1.233", "2.1.88"));
        assert!(!is_newer("2.1.88", "2.1.233"));
        assert!(is_newer("2.1.0", "0.2.30"));
    }

    fn hit(version: &str, surface: &str, excerpt: &str) -> Hit {
        Hit {
            version: version.into(),
            token: "t".into(),
            surface: surface.into(),
            owner: "o".into(),
            check: "c".into(),
            excerpt: excerpt.into(),
        }
    }

    #[test]
    fn hits_collapse_to_one_entry_per_surface_newest_first() {
        let hits = vec![
            hit("2.1.100", "env", "a"),
            hit("2.1.233", "env", "b"),
            hit("2.1.120", "models", "c"),
        ];
        let g = group_hits(&hits);
        assert_eq!(g.len(), 2);
        assert_eq!(g[0].surface, "env", "newest surface sorts first");
        assert_eq!(g[0].mentions, 2);
        assert_eq!(g[0].newest, "2.1.233");
    }

    /// One bullet naming three env vars is one fact. Without the dedupe
    /// the examples block repeats the same sentence three times and
    /// crowds out the other two releases.
    #[test]
    fn the_same_bullet_is_shown_once() {
        let hits = vec![
            hit("2.1.233", "env", "same bullet"),
            hit("2.1.233", "env", "same bullet"),
            hit("2.1.232", "env", "other bullet"),
        ];
        let g = group_hits(&hits);
        assert_eq!(g[0].mentions, 3, "the count still reflects every hit");
        assert_eq!(g[0].examples.len(), 2, "but examples are deduped");
    }

    /// A cap that does not announce itself reads as completeness — the
    /// exact failure this whole routine exists to avoid.
    #[test]
    fn examples_are_capped_but_the_count_reveals_the_remainder() {
        let hits: Vec<Hit> = (0..10)
            .map(|i| hit("2.1.233", "env", &format!("bullet {i}")))
            .collect();
        let g = group_hits(&hits);
        assert_eq!(g[0].examples.len(), EXAMPLES_PER_SURFACE);
        assert_eq!(g[0].mentions, 10, "so the report can print '+7 more'");
    }

    /// Regression guard for the two tokens that shipped in the first
    /// draft of the watchlist: `plan` and `policy` matched ordinary
    /// English and produced 51 false hits over 115 releases. A token
    /// that matches prose trains the reader to skim.
    #[test]
    fn committed_tokens_are_distinctive_not_english_words() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let md = std::fs::read_to_string(root.join(WATCHLIST_REL)).unwrap();
        let prose = "we plan to revisit the policy for this session and \
                     resume the model once the update lands";
        for row in parse_watchlist(&md).unwrap() {
            for token in &row.tokens {
                assert!(
                    !token_matches(token, prose),
                    "watchlist token {token:?} (row {:?}) matches ordinary prose",
                    row.surface
                );
            }
        }
    }

    fn pin(artifact: &str, value: &str, kind: PinKind) -> Pin {
        Pin {
            artifact: artifact.into(),
            field: "f".into(),
            value: value.into(),
            kind,
            when_stale: "w".into(),
        }
    }

    #[test]
    fn pins_matching_the_installed_version_are_not_reported() {
        let pins = vec![
            pin("a", "2.1.233", PinKind::Version),
            pin("b", "2.1.88", PinKind::Version),
        ];
        let stale = compare_pins("2.1.233", &pins);
        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].pin.artifact, "b");
    }

    /// A date can never equal a version, so comparing freshness
    /// evidence against the installed CC produced a permanent "BEHIND"
    /// row carrying no information. Noise is what teaches a reader to
    /// skim a report they are supposed to act on.
    #[test]
    fn freshness_pins_are_never_reported_as_behind_a_version() {
        let pins = vec![
            pin("cc-env-evidence.json", "2026-07-28", PinKind::Freshness),
            pin("cc-env-evidence.json", "2026-04-01", PinKind::Freshness),
            pin("parity", "2.1.88", PinKind::Version),
        ];
        let stale = compare_pins("2.1.233", &pins);
        assert_eq!(stale.len(), 1, "only the version pin may be compared");
        assert_eq!(stale[0].pin.artifact, "parity");
    }

    /// The defect this guards: with no baseline, `since` used to fall
    /// back to the installed version, which makes `is_newer` false for
    /// every release. The scan then printed "0 releases newer" — a
    /// missing pin file rendering as a clean run.
    #[test]
    fn an_absent_baseline_cannot_masquerade_as_zero_new_releases() {
        let rows = parse_watchlist(TABLE).unwrap();
        let log = "## 2.1.233\n\n- Changed `cleanupPeriodDays` handling\n";
        // What the old fallback did:
        assert!(
            scan_changelog(&rows, log, "2.1.233").is_empty(),
            "since == installed hides every release — this is why the \
             fallback was removed rather than made smarter"
        );
        // With a real baseline the same input is not silent.
        assert!(!scan_changelog(&rows, log, "2.1.88").is_empty());
    }

    #[test]
    fn base64_round_trips_the_shapes_gh_returns() {
        for (enc, want) in [
            ("aGVsbG8=", "hello"),
            ("aGVsbG8h", "hello!"),
            ("IyBDaGFuZ2Vsb2cK", "# Changelog\n"),
        ] {
            assert_eq!(
                String::from_utf8(base64_decode(enc).unwrap()).unwrap(),
                want
            );
        }
    }

    /// `gh` returns the payload wrapped across lines; whitespace must be
    /// stripped before decoding or the whole changelog is garbage.
    #[test]
    fn base64_ignores_embedded_newlines() {
        let wrapped = "aGVs\nbG8=";
        let cleaned: String = wrapped.chars().filter(|c| !c.is_whitespace()).collect();
        assert_eq!(
            String::from_utf8(base64_decode(&cleaned).unwrap()).unwrap(),
            "hello"
        );
    }

    /// The real committed watchlist must parse. This is what makes
    /// "adding a CC-facing module without a row is a review finding"
    /// mechanically true rather than aspirational.
    #[test]
    fn the_committed_watchlist_parses() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let md = std::fs::read_to_string(root.join(WATCHLIST_REL)).unwrap();
        let rows = parse_watchlist(&md).unwrap();
        assert!(
            rows.len() >= 15,
            "watchlist shrank to {} rows — surfaces are not supposed to leave it",
            rows.len()
        );
        assert!(
            rows.iter().any(|r| r.surface.contains("cleanupPeriodDays")),
            "the worked example must stay in the list"
        );
    }
}
