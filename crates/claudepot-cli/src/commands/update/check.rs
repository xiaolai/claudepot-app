use crate::AppContext;
use anyhow::Result;
use claudepot_core::updates::settings_bridge;
use claudepot_core::updates::state::UpdateState;
use claudepot_core::updates::{
    compare_versions, fetch_cli_latest, fetch_desktop_latest, Channel,
};
use std::cmp::Ordering;

pub async fn run(ctx: &AppContext) -> Result<()> {
    let snapshot = super::collect_status();
    let cc = settings_bridge::read().unwrap_or_default();

    // CC's own setting wins; default to "latest" when unset.
    let channel = cc
        .auto_updates_channel
        .as_deref()
        .and_then(|s| s.parse::<Channel>().ok())
        .unwrap_or(Channel::Latest);

    // Probe upstream — best-effort. Network failure is surfaced in the
    // output but doesn't fail the command.
    let cli_latest = fetch_cli_latest(channel).await.ok();
    let desktop_latest = fetch_desktop_latest().await.ok();

    // Persist whatever we learned. State-save errors are intentionally
    // swallowed: the user asked for a probe, not a state mutation.
    let mut state = UpdateState::load();
    let now = chrono::Utc::now().timestamp();
    if let Some(v) = cli_latest.clone() {
        state.cache.cli.last_check_unix = Some(now);
        state.cache.cli.last_error = None;
        match channel {
            Channel::Latest => state.cache.cli.last_known_latest = Some(v),
            Channel::Stable => state.cache.cli.last_known_stable = Some(v),
        }
    } else {
        state.cache.cli.last_error = Some("network probe failed".into());
    }
    if let Some(d) = desktop_latest.as_ref() {
        state.cache.desktop.last_check_unix = Some(now);
        state.cache.desktop.last_error = None;
        state.cache.desktop.last_known_latest = Some(d.version.clone());
        state.cache.desktop.last_known_sha = d.commit_sha.clone();
    } else {
        state.cache.desktop.last_error = Some("network probe failed".into());
    }
    let _ = state.save();

    if ctx.json {
        println!(
            "{}",
            serde_json::json!({
                "cli": {
                    "channel": channel.as_str(),
                    "installs": snapshot.cli_installs,
                    "latest_remote": cli_latest,
                    "cc_settings": cc,
                },
                "desktop": {
                    "install": snapshot.desktop,
                    "running": snapshot.desktop_running,
                    "latest_remote": desktop_latest.as_ref().map(|d| serde_json::json!({
                        "version": d.version,
                        "commit_sha": d.commit_sha,
                    })),
                },
            })
        );
        return Ok(());
    }

    println!("CC CLI");
    println!("  Channel: {}", channel.as_str());
    match cli_latest.as_deref() {
        Some(v) => println!("  Latest:  {v}"),
        None => println!("  Latest:  <network probe failed>"),
    }
    if cc.disable_updates {
        println!("  ⚠  DISABLE_UPDATES=1 set in ~/.claude/settings.json");
    }
    if cc.disable_autoupdater {
        println!("  ⚠  DISABLE_AUTOUPDATER=1 set in ~/.claude/settings.json");
    }
    if let Some(p) = cc.minimum_version.as_deref() {
        println!("  Floor:   minimumVersion = {p}");
    }
    if snapshot.cli_installs.is_empty() {
        println!("  Detected installs: (none)");
    } else {
        println!("  Detected installs:");
        for c in &snapshot.cli_installs {
            let comparison = match (c.version.as_deref(), cli_latest.as_deref()) {
                (Some(have), Some(want)) => match compare_versions(have, want) {
                    Ordering::Less => "  ← UPDATE AVAILABLE",
                    Ordering::Equal => "  ✓ up to date",
                    Ordering::Greater => "  (newer than channel)",
                },
                _ => "",
            };
            println!("    • {}{}", super::cli_install_summary(c), comparison);
        }
    }

    println!();
    println!("Claude Desktop");
    if let Some(d) = snapshot.desktop.as_ref() {
        println!("  Installed: {}", super::desktop_install_summary(d));
        println!(
            "  Running:   {}",
            if snapshot.desktop_running { "yes" } else { "no" }
        );
        match desktop_latest.as_ref() {
            Some(latest) => {
                println!("  Latest:    {}", latest.version);
                if let Some(have) = d.version.as_deref() {
                    match compare_versions(have, &latest.version) {
                        Ordering::Less => println!("  ← UPDATE AVAILABLE"),
                        Ordering::Equal => println!("  ✓ up to date"),
                        Ordering::Greater => println!("  (newer than upstream)"),
                    }
                }
            }
            None => println!("  Latest:    <network probe failed>"),
        }
    } else {
        println!("  No Claude Desktop install detected.");
    }

    Ok(())
}
