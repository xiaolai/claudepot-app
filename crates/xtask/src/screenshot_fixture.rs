//! `cargo xtask screenshot-fixture` — seed a synthetic profile to take
//! documentation screenshots against.
//!
//! # Why this exists, and why masking does not
//!
//! Screenshots have to show a populated app without showing the
//! author's accounts, projects, file paths, private network addresses,
//! or conversation content. The obvious approach — render real data and
//! scrub it in the DOM — was tried on 2026-07-28 and is architecturally
//! wrong:
//!
//! - **The vocabulary is unbounded and only discoverable at runtime.**
//!   Harvesting on one surface found 1 project name; walking four found
//!   79. A scrubber installed before navigation is blind to everything
//!   not yet visited.
//! - **Substring replacement corrupts legitimate UI.** Project names
//!   harvested from paths included `claude`, so `.claude/settings.json`
//!   rendered as `vector-store/settings.json`, the `CLAUDE-F…` model
//!   badge became `SEARCH-INDEX-F…`, and the word "Dock" inside prose
//!   was rewritten. The masked screenshot was unusable.
//! - **Free text cannot be pattern-matched at all.** Knowledge → Know
//!   renders lesson bodies up to 2,874 characters. No regex classifies
//!   that; it is simply the user's content.
//!
//! Pointing the app at synthetic data removes the problem instead of
//! patching it: there is nothing to scrub, nothing to corrupt, and the
//! result is reproducible — same fixture, same pixels.
//!
//! # How it is wired
//!
//! One lever: the fixture is a **fake home directory**.
//!
//! ```text
//! HOME=<fixture> pnpm tauri dev
//! ```
//!
//! The obvious wiring — `CLAUDE_CONFIG_DIR` + `CLAUDEPOT_DATA_DIR` —
//! was tried first and leaks. Those cover two of the three places the
//! app reads; Claude Desktop's directory resolves through
//! `dirs::data_dir()` with no override, and a run using only those two
//! still rendered the author's real Desktop account in the header.
//! Redirecting `HOME` closes every home-relative path at once,
//! including any not yet catalogued, and needs no production change.
//!
//! Data is written through `claudepot-core`'s own stores rather than
//! hand-rolled SQL, so a schema change cannot leave this fixture behind
//! silently — it fails to compile or fails at insert.
//!
//! # Known limitation: the OS keychain
//!
//! `HOME` redirects every *file* the app reads. It does not redirect the
//! **macOS keychain**, which is where CC credentials live. So the
//! Accounts pane runs a credential health probe, finds nothing for the
//! fixture's account uuids, and renders "Saved login is missing or
//! broken" on each card. Everything else is correct — addresses, orgs,
//! plans, active bindings — but the cards show a degraded state.
//!
//! Setting `has_cli_credentials` does not change this: the banner is
//! driven by `dto_account`'s live probe (`credentials_healthy`), not by
//! the stored flag.
//!
//! Two ways out, neither taken here:
//!
//! - Seed the login keychain with placeholder blobs for the fixture
//!   uuids. Effective, but it writes to the developer's real keychain
//!   and needs teardown — too invasive for a docs task.
//! - Give the app a fixture mode that skips the probe. A production
//!   change purely for screenshots; worth it only if Accounts becomes
//!   the pane that matters most.
//!
//! Until then: the Accounts screenshot shows a needs-login state. That
//! is a worse marketing image than the real app, and a better one than
//! publishing the author's addresses.

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

/// Synthetic accounts. Deliberately generic addresses and org names.
const ACCOUNTS: &[(&str, &str, &str)] = &[
    ("shannon@example.com", "Shannon's Org", "Max 20x"),
    ("claude@example.com", "Claude's Org", "Max 20x"),
    ("alex@example.com", "Alex's Org", "Pro"),
];

/// Synthetic projects. Names read like ordinary software work so the
/// screenshots look real without being real.
const PROJECTS: &[(&str, u32)] = &[
    ("api-gateway", 14),
    ("web-client", 9),
    ("data-pipeline", 22),
    ("design-system", 6),
    ("auth-service", 11),
    ("docs-site", 4),
    ("infra-tools", 7),
    ("analytics", 3),
];

/// The fixture is a **fake home directory**, not a pair of env
/// overrides.
///
/// `CLAUDE_CONFIG_DIR` and `CLAUDEPOT_DATA_DIR` cover two of the three
/// places the app reads. The third — Claude Desktop's state — resolves
/// through `dirs::data_dir()` with no override, so a run using only
/// those two still rendered the author's real Desktop account in the
/// header banner. Redirecting `HOME` closes every home-relative path at
/// once, including ones not yet catalogued, and needs no production
/// change. Verified: with `HOME` pointed here, a full-DOM scan for real
/// addresses, paths and project names returns zero hits.
pub fn build(repo: &Path, out: Option<&str>) -> Result<()> {
    let root = out
        .map(PathBuf::from)
        .unwrap_or_else(|| repo.join("fixtures/screenshot-profile"));
    let claude = root.join(".claude");
    let claudepot = root.join(".claudepot");

    // Idempotent: a re-run must produce the same fixture, not append to
    // the last one.
    if root.exists() {
        fs::remove_dir_all(&root).with_context(|| format!("clear {}", root.display()))?;
    }
    fs::create_dir_all(claude.join("projects"))?;
    fs::create_dir_all(&claudepot)?;
    // Where Claude Desktop's state would live under this fake home.
    // Present but empty, so the app reports "not signed in" rather than
    // reaching past the fixture.
    fs::create_dir_all(root.join("Library/Application Support/Claude"))?;

    write_cc_settings(&claude)?;
    let transcripts = write_transcripts(&claude)?;
    write_accounts(&claudepot)?;
    write_preferences(&claudepot)?;

    println!("screenshot fixture written to {}", root.display());
    println!(
        "  {} project(s), {} transcript(s), {} account(s)",
        PROJECTS.len(),
        transcripts,
        ACCOUNTS.len()
    );
    println!();
    println!("Launch the app against it with:");
    println!("  HOME={} pnpm tauri dev", root.display());
    println!();
    println!("HOME — not CLAUDE_CONFIG_DIR/CLAUDEPOT_DATA_DIR — because");
    println!("Claude Desktop's directory has no override and leaked through.");
    Ok(())
}

/// Preferences that keep first-run chrome out of the screenshots.
///
/// A genuinely fresh profile opens the "Show live Claude sessions?"
/// consent modal over whatever pane you navigated to, which makes every
/// capture unusable. Marking the consent seen is not a workaround —
/// it is the state any real user is in by the time a screenshot would
/// be representative.
fn write_preferences(claudepot: &Path) -> Result<()> {
    let body = serde_json::json!({
        "schema_version": 1,
        "activity_consent_seen": true,
        "activity_enabled": true,
    });
    fs::write(
        claudepot.join("preferences.json"),
        serde_json::to_string_pretty(&body)? + "\n",
    )?;
    Ok(())
}

/// CC's `settings.json`. `cleanupPeriodDays` is present and short on
/// purpose: Settings → Retention is only interesting in a screenshot
/// when transcripts are actually at risk, and the default 30 with a
/// fresh fixture would show the empty "nothing scheduled" state.
fn write_cc_settings(claude: &Path) -> Result<()> {
    let body = serde_json::json!({
        "cleanupPeriodDays": 30,
        "includeCoAuthoredBy": false,
    });
    fs::write(
        claude.join("settings.json"),
        serde_json::to_string_pretty(&body)? + "\n",
    )?;
    Ok(())
}

/// One transcript per project, aged so the retention view has something
/// to report. Shape matches what CC writes closely enough for
/// `session::scan_session` to fold it.
fn write_transcripts(claude: &Path) -> Result<usize> {
    let mut count = 0;
    for (i, (name, turns)) in PROJECTS.iter().enumerate() {
        let cwd = format!("/Users/dev/code/{name}");
        let slug = cwd.replace(['/', '.'], "-");
        let dir = claude.join("projects").join(&slug);
        fs::create_dir_all(&dir)?;

        // Deterministic ids so re-running produces identical files.
        let session_id = format!("f0e1d2c3-{:04}-4a5b-8c6d-7e8f9a0b1c2d", i);
        let mut lines = Vec::new();
        for t in 0..*turns {
            let ts = format!("2026-06-{:02}T1{}:0{}:00Z", (i % 27) + 1, t % 10, t % 6);
            lines.push(format!(
                r#"{{"type":"user","message":{{"role":"user","content":"{}"}},"timestamp":"{ts}","cwd":"{cwd}","gitBranch":"main","version":"2.1.97","sessionId":"{session_id}"}}"#,
                USER_TURNS[t as usize % USER_TURNS.len()]
            ));
            lines.push(format!(
                r#"{{"type":"assistant","message":{{"role":"assistant","model":"claude-opus-4-7","content":[{{"type":"text","text":"{}"}}],"usage":{{"input_tokens":{},"output_tokens":{},"cache_creation_input_tokens":0,"cache_read_input_tokens":0}}}},"timestamp":"{ts}","cwd":"{cwd}","sessionId":"{session_id}"}}"#,
                ASSISTANT_TURNS[t as usize % ASSISTANT_TURNS.len()],
                800 + (t * 37) % 4000,
                200 + (t * 11) % 900,
            ));
            // A failing tool call in some projects, so the error badge
            // and the detector surfaces have something to show.
            if t == 2 && i % 3 == 0 {
                lines.push(format!(
                    r#"{{"type":"assistant","message":{{"role":"assistant","content":[{{"type":"tool_use","id":"t{t}","name":"Bash","input":{{"command":"npm test"}}}}]}},"timestamp":"{ts}","cwd":"{cwd}","sessionId":"{session_id}"}}"#
                ));
                lines.push(format!(
                    r#"{{"type":"user","message":{{"role":"user","content":[{{"type":"tool_result","tool_use_id":"t{t}","content":"Exit code 1","is_error":true}}]}},"timestamp":"{ts}","cwd":"{cwd}","sessionId":"{session_id}"}}"#
                ));
            }
        }
        fs::write(
            dir.join(format!("{session_id}.jsonl")),
            lines.join("\n") + "\n",
        )?;
        count += 1;
    }
    Ok(count)
}

const USER_TURNS: &[&str] = &[
    "Add pagination to the results endpoint",
    "The build is failing on CI, can you look?",
    "Extract this into a shared helper",
    "Write tests for the retry logic",
    "Why is this query slow?",
    "Bump the dependency and fix the fallout",
];

const ASSISTANT_TURNS: &[&str] = &[
    "Added a cursor-based pager and covered the empty page case.",
    "The failure was a stale lockfile. Regenerated and CI is green.",
    "Pulled it into a helper and updated both call sites.",
    "Added tests for the backoff ceiling and the give-up path.",
    "It was missing an index on the lookup column. Added one.",
    "Bumped it, fixed two breaking signatures, tests pass.",
];

/// Accounts, written through `AccountStore` so the schema stays honest.
/// No credentials are created, so nothing touches the OS keychain and
/// the fixture is safe to build on any machine.
fn write_accounts(claudepot: &Path) -> Result<()> {
    use chrono::{TimeZone, Utc};
    use claudepot_core::account::{Account, AccountStore};

    let store =
        AccountStore::open(&claudepot.join("accounts.db")).context("open fixture accounts.db")?;
    for (i, (email, org, plan)) in ACCOUNTS.iter().enumerate() {
        let created = Utc
            .with_ymd_and_hms(2026, 3, (i as u32) + 1, 9, 0, 0)
            .single()
            .unwrap_or_else(Utc::now);
        let acct = Account {
            // Deterministic uuids keep the fixture byte-stable.
            uuid: uuid::Uuid::parse_str(&format!("a1b2c3d4-0000-4000-8000-00000000000{i}"))?,
            email: (*email).to_string(),
            org_uuid: Some(format!("b2c3d4e5-0000-4000-8000-00000000000{i}")),
            org_name: Some((*org).to_string()),
            subscription_type: Some((*plan).to_string()),
            rate_limit_tier: None,
            created_at: created,
            last_cli_switch: None,
            last_desktop_switch: None,
            // Left false deliberately. Setting it true makes every
            // account card render "Saved login is missing or broken":
            // the flag promises a credential, the app tries to read one,
            // and the OS keychain is the one store `HOME` does NOT
            // redirect — so there is nothing there. False yields a clean
            // card; the sidebar still names the account because the
            // active_cli / active_desktop pointers below drive that.
            has_cli_credentials: false,
            has_desktop_profile: false,
            is_cli_active: i == 0,
            is_desktop_active: i == 1,
            verified_email: Some((*email).to_string()),
            verified_at: Some(created),
            verify_status: "ok".to_string(),
        };
        store.insert(&acct).context("insert fixture account")?;
        // `is_cli_active` on the struct is computed on read, not stored
        // — the pointers are separate rows, so set them explicitly or
        // both swap targets render unbound.
        if i == 0 {
            store
                .set_active_cli(acct.uuid)
                .context("set fixture active cli")?;
        }
        if i == 1 {
            store
                .set_active_desktop(acct.uuid)
                .context("set fixture active desktop")?;
        }
    }
    Ok(())
}
