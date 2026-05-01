use crate::AppContext;
use anyhow::Result;
use claudepot_core::updates::detect::detect_cli_installs;
use claudepot_core::updates::settings_bridge;
use claudepot_core::updates::state::UpdateState;

// One arg per toggle is the right shape here — the CLI parser
// hands them through individually because each maps to a distinct
// `--cli-notify` / `--desktop-auto` / etc. flag, and bundling them
// into a config struct would just hide the trivial pass-through.
#[allow(clippy::too_many_arguments)]
pub async fn run(
    ctx: &AppContext,
    channel: Option<String>,
    allow_downgrade: bool,
    cli_notify: Option<bool>,
    cli_notify_os: Option<bool>,
    desktop_notify: Option<bool>,
    desktop_notify_os: Option<bool>,
    desktop_auto: Option<bool>,
) -> Result<()> {
    let mut wrote = false;

    if let Some(c) = channel.as_deref() {
        // Route through change_channel so the CLI honors the same
        // CC-compatible minimumVersion semantics the GUI does
        // (latest→stable pins by default; --allow-downgrade clears
        // the floor for explicit opt-in; stable→latest always clears).
        let installed = detect_cli_installs()
            .into_iter()
            .find(|c| c.is_active)
            .and_then(|c| c.version);
        settings_bridge::change_channel(c, installed.as_deref(), allow_downgrade)
            .map_err(|e| anyhow::anyhow!("{e}"))?;
        let suffix = if c == "stable" && allow_downgrade {
            " (downgrade allowed: minimumVersion cleared)"
        } else if c == "stable" && installed.is_some() {
            " (pinned to current; pass --allow-downgrade to clear floor)"
        } else {
            ""
        };
        ctx.info(&format!(
            "✓ Set autoUpdatesChannel = {c} in ~/.claude/settings.json{suffix}"
        ));
        wrote = true;
    }

    if cli_notify.is_some()
        || cli_notify_os.is_some()
        || desktop_notify.is_some()
        || desktop_notify_os.is_some()
        || desktop_auto.is_some()
    {
        let mut state = UpdateState::load();
        if let Some(b) = cli_notify {
            state.settings.cli.notify_on_available = b;
        }
        if let Some(b) = cli_notify_os {
            state.settings.cli.notify_os_on_available = b;
        }
        if let Some(b) = desktop_notify {
            state.settings.desktop.notify_on_available = b;
        }
        if let Some(b) = desktop_notify_os {
            state.settings.desktop.notify_os_on_available = b;
        }
        if let Some(b) = desktop_auto {
            state.settings.desktop.auto_install_when_quit = b;
        }
        state.save().map_err(|e| anyhow::anyhow!("{e}"))?;
        ctx.info("✓ Wrote ~/.claudepot/updates.json");
        wrote = true;
    }

    let cc = settings_bridge::read().unwrap_or_default();
    let state = UpdateState::load();

    if ctx.json {
        println!(
            "{}",
            serde_json::json!({
                "cc_settings": cc,
                "settings": state.settings,
                "wrote": wrote,
            })
        );
        return Ok(());
    }

    println!("Update settings");
    println!();
    println!("CC (~/.claude/settings.json)");
    println!(
        "  autoUpdatesChannel    : {}",
        cc.auto_updates_channel
            .as_deref()
            .unwrap_or("(unset, default 'latest')")
    );
    println!(
        "  minimumVersion        : {}",
        cc.minimum_version.as_deref().unwrap_or("(unset)")
    );
    println!("  DISABLE_AUTOUPDATER   : {}", cc.disable_autoupdater);
    println!("  DISABLE_UPDATES       : {}", cc.disable_updates);
    println!();
    println!("Claudepot (~/.claudepot/updates.json)");
    println!(
        "  cli.notify_on_available       : {}",
        state.settings.cli.notify_on_available
    );
    println!(
        "  cli.notify_os_on_available    : {}",
        state.settings.cli.notify_os_on_available
    );
    println!(
        "  cli.force_update_on_check     : {}",
        state.settings.cli.force_update_on_check
    );
    println!(
        "  desktop.notify_on_available   : {}",
        state.settings.desktop.notify_on_available
    );
    println!(
        "  desktop.notify_os_on_available: {}",
        state.settings.desktop.notify_os_on_available
    );
    println!(
        "  desktop.auto_install_when_quit: {}",
        state.settings.desktop.auto_install_when_quit
    );
    println!(
        "  poll_interval_minutes       : {}",
        state.poll_interval_minutes()
    );

    Ok(())
}
