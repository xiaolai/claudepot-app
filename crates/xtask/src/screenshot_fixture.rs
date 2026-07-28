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
//! pnpm dev &                                    # vite, real HOME
//! cargo build -p claudepot-tauri                # real HOME
//! HOME=<fixture> ./target/debug/claudepot-tauri # app only
//! ```
//!
//! The fake home goes to the **app**, not the build. `HOME=<fixture>
//! pnpm tauri dev` looks tidier and fails: rustup reads its default
//! toolchain from `$HOME/.rustup`, so the override takes the toolchain
//! with it and `cargo metadata` dies before the app is ever compiled.
//! (Corepack's cache moves too, prompting a pnpm re-download.)
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
use claudepot_core::session_index::SessionIndex;
use claudepot_core::shared_memory::durable::{
    self, CreatedByKind, MemoryKind, NewMemory, NewProposal, Scope,
};
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

/// Where the fake home lives — **outside the repo, on purpose**.
///
/// The first version put it at `<repo>/fixtures/screenshot-profile`,
/// which leaks: the app legitimately displays the paths it reads, so
/// Global → Config rendered
/// `Config home /Users/<you>/…/claudepot-app/fixtures/…/.claude` in
/// full, and Global → Memory rendered the same prefix truncated. A
/// fixture whose own path embeds the author's home cannot produce a
/// leak-free screenshot no matter what data is inside it.
///
/// `/tmp/claudepot-demo-home` contains no username, no repo name, and no
/// directory structure of the author's — and it reads honestly to a
/// documentation reader as what it is: a demo profile.
fn default_root() -> PathBuf {
    #[cfg(unix)]
    {
        // Not `env::temp_dir()`: on macOS that is the opaque per-user
        // `/var/folders/<hash>/T/`, which looks like debris in a
        // screenshot even though it leaks nothing.
        PathBuf::from("/tmp/claudepot-demo-home")
    }
    #[cfg(not(unix))]
    {
        std::env::temp_dir().join("claudepot-demo-home")
    }
}

/// Lessons a human already reviewed and kept — `review_state =
/// 'accepted'`. Without them Knowledge screenshots as "No lessons yet",
/// which documents the empty state rather than the feature.
///
/// The text is written to read like real engineering knowledge, because
/// a screenshot of lorem ipsum teaches a reader nothing about what the
/// pane is for. It describes the synthetic projects only.
const LESSONS: &[(&str, MemoryKind, &str)] = &[
    (
        "api-gateway",
        MemoryKind::Constraint,
        "Rate-limit headers must be written before the response body starts \
         streaming. Setting them from the after-response hook compiles and \
         passes tests, then silently drops them under HTTP/2.",
    ),
    (
        "api-gateway",
        MemoryKind::Pattern,
        "Every upstream call goes through the retry wrapper, never through \
         the bare client. The wrapper is where the circuit breaker and the \
         request-id propagation live.",
    ),
    (
        "auth-service",
        MemoryKind::Fact,
        "Refresh tokens rotate on every use. A 401 from /profile means the \
         stored blob is one generation behind — not that the account was \
         signed out. Re-exchange before surfacing an error to the user.",
    ),
    (
        "data-pipeline",
        MemoryKind::Pattern,
        "Batch jobs are idempotent by partition key: a retried run overwrites \
         its partition rather than appending. A job that appends without a \
         dedupe key will double-count on any retry.",
    ),
    (
        "design-system",
        MemoryKind::Preference,
        "Design tokens are declared in exactly one file. A component that \
         hardcodes a colour or a radius is a review finding, not a style \
         choice — add the semantic token first, then reference it.",
    ),
    (
        "infra-tools",
        MemoryKind::Constraint,
        "Never run the schema push command against production: it diffs the \
         database against the schema files and drops anything it cannot see, \
         including generated columns and triggers.",
    ),
];

/// Distiller output awaiting human review — `review_state = 'proposed'`.
/// Gives Knowledge → Review a non-empty queue, which is the whole point
/// of that tab: claims arrive proposed and a human accepts or rejects.
///
/// `(project, content, directive, confidence)`
const PROPOSALS: &[(&str, &str, i64)] = &[
    (
        "web-client",
        "Suspense boundaries belong at the route level, not around individual \
         data hooks. Wrapping each hook produced a cascade of spinners on \
         first paint.",
        72,
    ),
    (
        "data-pipeline",
        "The nightly export appears to assume UTC while the ingest timestamps \
         are local, which would shift one hour of rows across the day \
         boundary twice a year.",
        58,
    ),
    (
        "docs-site",
        "Code samples in the guide drifted from the API after the v3 rename; \
         the samples are not covered by any test that would have caught it.",
        64,
    ),
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
pub fn build(_repo: &Path, out: Option<&str>) -> Result<()> {
    let root = out.map(PathBuf::from).unwrap_or_else(default_root);
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
    let (accepted, proposed) = seed_knowledge(&claudepot)?;

    println!("screenshot fixture written to {}", root.display());
    println!(
        "  {} project(s), {} transcript(s), {} account(s)",
        PROJECTS.len(),
        transcripts,
        ACCOUNTS.len()
    );
    println!("  {accepted} accepted lesson(s), {proposed} awaiting review");
    println!();
    println!("Launch the app against it with:");
    println!("  pnpm dev &                                     # vite, real HOME");
    println!("  cargo build -p claudepot-tauri                 # real HOME");
    println!("  HOME={} ./target/debug/claudepot-tauri", root.display());
    println!();
    println!("The fake home goes to the APP, not the build: rustup reads its");
    println!("default toolchain from $HOME/.rustup, so `HOME=… pnpm tauri dev`");
    println!("kills cargo before the app compiles.");
    println!();
    println!("HOME — not CLAUDE_CONFIG_DIR/CLAUDEPOT_DATA_DIR — because");
    println!("Claude Desktop's directory has no override and leaked through.");
    Ok(())
}

/// Seed the Knowledge pane's lessons into `sessions.db`.
///
/// Written through `claudepot-core`'s own writers — `create_memory` for
/// accepted rows, `create_proposal` for the review queue — rather than
/// hand-rolled SQL. That is not a style preference: `create_proposal`
/// inserts `review_state = 'proposed'` in a single statement precisely
/// so no crash window can leave an agent-authored claim ACCEPTED. A
/// fixture that INSERTed directly would be free to produce a row shape
/// production can never reach, and the screenshot would document a state
/// that does not exist.
///
/// Memories are keyed by `project_path` text and carry no FK to the
/// session rows, so the app's first index refresh — which reaps rows for
/// files it cannot find — leaves them alone.
fn seed_knowledge(claudepot: &Path) -> Result<(usize, usize)> {
    let idx = SessionIndex::open(&claudepot.join("sessions.db"))
        .map_err(|e| anyhow::anyhow!("open sessions.db: {e}"))?;

    for (project, kind, content) in LESSONS {
        let path = project_path(project);
        durable::create_memory(
            &idx,
            &NewMemory {
                scope: Scope::Project,
                project_path: Some(&path),
                kind: *kind,
                content,
                created_by_kind: CreatedByKind::User,
                created_by: "fixture",
                confidence: None,
            },
        )
        .map_err(|e| anyhow::anyhow!("seed lesson for {project}: {e}"))?;
    }

    for (project, content, confidence) in PROPOSALS {
        let path = project_path(project);
        durable::create_proposal(
            &idx,
            &NewProposal {
                project_path: &path,
                kind: MemoryKind::Pattern,
                content,
                // The one-line instruction a future session would act on;
                // the Review queue renders it beside the claim.
                directive: "Review this claim before relying on it.",
                confidence: *confidence,
                anchor_json: None,
                origin_exchange_id: None,
                origin_file_path: None,
                created_by: "fixture-distiller",
            },
        )
        .map_err(|e| anyhow::anyhow!("seed proposal for {project}: {e}"))?;
    }

    Ok((LESSONS.len(), PROPOSALS.len()))
}

/// The cwd `write_transcripts` records for a project. One helper so the
/// lesson rows and the transcripts agree — a mismatch would render
/// lessons attached to projects the Projects pane does not list.
fn project_path(project: &str) -> String {
    format!("/Users/dev/code/{project}")
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
        let cwd = project_path(name);
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
