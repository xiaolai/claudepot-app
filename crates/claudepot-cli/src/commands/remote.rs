//! `claudepot remote …` — the LAN appliance surface.
//!
//! Thin over [`claudepot_core::remote::service`], which is the one
//! implementation of every verb here. The GUI's Settings → Remote pane
//! calls the same functions; what stays here is **presentation** — the
//! stdin password convention, the `--json` shapes, and the paragraph
//! about `0.0.0.0` that a pane renders as a banner instead.
//!
//! Passwords are read from **stdin**, following the convention
//! `account add` already set — this CLI deliberately does not ship
//! `rpassword`. Typing one interactively echoes it, so the piped form
//! is the documented one.

use std::sync::Arc;

use anyhow::{Context, Result};
use claudepot_core::remote::bind::Exposure;
use claudepot_core::remote::server::{router, AppState};
use claudepot_core::remote::service::{self, exposure_word, ApprovalHook, FilePersist};
use claudepot_core::remote::{approval, config, serve, store as device_store};
use tokio::sync::Mutex;

use crate::output::print_json;
use crate::AppContext;

/// Never silent: the throttle counter and the spent-TOTP high-water
/// mark were in that file.
fn warn_if_recovered(status: &service::Status) {
    if status.config_recovered {
        eprintln!(
            "warning: the remote config was unreadable and has been reset — \
             the login throttle and TOTP replay guard start from zero, and \
             the surface is disabled until reconfigured."
        );
    }
    if status.devices_recovered {
        eprintln!(
            "warning: the paired-device records were unreadable and have been reset — \
             every previous revocation is lost. Treat any token issued before now as live."
        );
    }
}

pub fn status_cmd(ctx: &AppContext) -> Result<()> {
    let st = service::status(approval::now_ms())?;
    warn_if_recovered(&st);

    if ctx.json {
        print_json(&serde_json::json!({
            "enabled": st.enabled,
            // Liveness, which `enabled` is not — see `service::Status`.
            "serving": st.serving,
            "bind": st.bind.to_string(),
            "port": st.port,
            "exposure": st.exposure.map(exposure_word),
            "password_set": st.password_set,
            "totp_enabled": st.totp_enabled,
            "passkeys": st.passkeys,
            "approvals_enabled": st.approvals_enabled,
            "requires_tls": st.requires_tls,
            "bind_error": st.bind_error,
            "devices_active": st.active_device_count(chrono::Utc::now()),
            "config_recovered": st.config_recovered,
            "devices_recovered": st.devices_recovered,
        }))?;
        return Ok(());
    }

    // Three states, not two. `enabled` survives a `kill -9`, so
    // "enabled but nothing is serving" is a real and reachable state,
    // and collapsing it into "ON" is the same defect the approval
    // hook's runtime gate exists to avoid.
    println!(
        "Remote surface: {}",
        match (st.enabled, st.serving) {
            (false, _) => "off",
            (true, true) => "ON, serving",
            (true, false) => "enabled, NOT serving (start it with `claudepot remote serve`)",
        }
    );
    println!("  bind      {}:{}", st.bind, st.port);
    // Who can actually reach that address. The line above names a
    // number; on its own it does not tell a reader whether their phone
    // will work from the office, and `bind`'s allowlist means the
    // answer is usually no. Stated here rather than left to be found
    // out from a phone that cannot connect.
    println!("  reach     {}", reach_line(&st.bind));
    match (&st.bind_error, st.exposure) {
        (Some(e), _) => println!("  bind      REFUSED: {e}"),
        (None, Some(Exposure::EveryInterface)) => {
            println!("  exposure  EVERY INTERFACE — see the warning under `enable`")
        }
        _ => {}
    }
    println!(
        "  tls       {}",
        if st.requires_tls {
            "required (not loopback)"
        } else {
            "not needed (loopback is already a secure context)"
        }
    );
    println!(
        "  password  {}",
        if st.password_set {
            "set"
        } else {
            "NOT SET — the surface will refuse every login"
        }
    );
    println!("  2FA       {}", if st.totp_enabled { "on" } else { "off" });
    println!("  passkeys  {}", st.passkeys);
    // Named as a capability rather than a setting, because that is what
    // it grants — see `RemoteAction::Approvals`.
    println!(
        "  approvals {}",
        if st.approvals_enabled {
            "ON — a paired device can approve a tool call"
        } else {
            "off — permission prompts are answered at the machine"
        }
    );
    println!(
        "  devices   {} active",
        st.active_device_count(chrono::Utc::now())
    );
    Ok(())
}

/// One line describing who can reach a bound address.
///
/// `remote::bind` refuses every globally routable address, so the
/// honest answer is "this network" unless the user has put their
/// devices on a mesh VPN — which the Tailscale CGNAT range is the
/// detectable case of. Nothing in the product said this: the only
/// mention of Tailscale was inside a bind *error*, reachable by typing
/// something wrong.
///
/// The port-forwarding warning is here rather than in the docs because
/// the user who wants access from anywhere and is told they cannot have
/// it is exactly the user who reaches for their router. The allowlist
/// stops Claudepot *binding* a public address; it cannot stop a router
/// forwarding a port to a private one, and that path puts the admin
/// password on the open internet.
fn reach_line(bind: &std::net::IpAddr) -> String {
    use claudepot_core::remote::bind::is_tailscale_range;
    use std::net::IpAddr;

    if bind.is_loopback() {
        return "this machine only".to_string();
    }
    if let IpAddr::V4(v4) = bind {
        if is_tailscale_range(*v4) {
            return "your tailnet, from anywhere — plus this network".to_string();
        }
    }
    "devices on this network only; for access from elsewhere join both \
     to a mesh VPN (Tailscale/Headscale) and bind that address. Never \
     port-forward this."
        .to_string()
}

pub fn set_password_cmd(ctx: &AppContext) -> Result<()> {
    ctx.info("Reading the new password from stdin...");
    let mut buf = String::new();
    std::io::stdin()
        .read_line(&mut buf)
        .context("read password from stdin")?;
    let mut plain = buf.trim_end_matches(['\n', '\r']).to_string();
    // The raw buffer is a second copy of the secret.
    use zeroize::Zeroize;
    buf.zeroize();

    // `set_password` zeroizes `plain` on every path, success or error.
    service::set_password(&mut plain)?;

    if ctx.json {
        print_json(&serde_json::json!({ "password_set": true }))?;
    } else {
        println!("Password set. Existing sessions are unaffected —");
        println!("revoke them with `claudepot remote revoke-all` if that is not what you want.");
    }
    Ok(())
}

pub fn enable_cmd(ctx: &AppContext, bind: Option<&str>, port: Option<u16>) -> Result<()> {
    let enabled = service::enable(bind, port)?;

    if ctx.json {
        print_json(&serde_json::json!({
            "enabled": true,
            "bind": enabled.bind.to_string(),
            "port": enabled.port,
            "requires_tls": enabled.requires_tls,
            "exposure": exposure_word(enabled.exposure),
        }))?;
        return Ok(());
    }
    println!(
        "Remote surface enabled on {}:{}.",
        enabled.bind, enabled.port
    );
    // `bind` accepts `0.0.0.0` deliberately, and core returns
    // `Exposure::EveryInterface` precisely so the caller has to say so —
    // "Accepted, never silently", in its words.
    if enabled.exposure == Exposure::EveryInterface {
        println!(
            "  Warning: this listens on EVERY interface. If this host \
             ever gets a public address, the admin password becomes the \
             only thing between the internet and code execution as you. \
             Bind a specific private address unless you need this."
        );
    }
    if enabled.requires_tls {
        println!("TLS is required for this address. Mint a certificate first:");
        println!("  ./scripts/mint-remote-cert.sh <hostname>");
    }
    println!("Start it with `claudepot remote serve`.");
    Ok(())
}

pub fn disable_cmd(ctx: &AppContext) -> Result<()> {
    service::disable()?;
    if ctx.json {
        print_json(&serde_json::json!({ "enabled": false }))?;
    } else {
        println!("Remote surface disabled.");
        // The preference is off; a server already running keeps
        // serving until it is stopped. Saying otherwise would be the
        // enabled-is-not-liveness confusion from the other side.
        println!("A server that is already running keeps serving until you stop it.");
    }
    Ok(())
}

pub fn approvals_cmd(ctx: &AppContext, enabled: bool) -> Result<()> {
    service::set_approvals(enabled)?;

    // A running server picks this up on its next hook invocation —
    // `approval::store::gate` reads the preference every time — so
    // there is nothing to restart. What this process cannot do is
    // uninstall the hook entry from a server it does not own; the gate
    // makes that entry inert, and `ApprovalHook::arm` clears it on the
    // next start.
    if ctx.json {
        print_json(&serde_json::json!({ "approvals_enabled": enabled }))?;
        return Ok(());
    }
    if enabled {
        println!("Approvals ON. A paired device can now approve a tool call —");
        println!("which is arbitrary code execution as you, behind the admin password.");
    } else {
        println!("Approvals off. Permission prompts are drawn at the machine as usual.");
        println!("The panel still lists sessions, reads transcripts and sends prompts.");
    }
    Ok(())
}

pub fn revoke_all_cmd(ctx: &AppContext) -> Result<()> {
    let n = service::revoke_all()?;
    if ctx.json {
        print_json(&serde_json::json!({ "revoked": n }))?;
    } else {
        println!("Revoked {n} session(s)/device(s).");
    }
    Ok(())
}

pub async fn serve_cmd(ctx: &AppContext) -> Result<()> {
    let loaded = config::load().context("load remote config")?;
    let cfg = loaded.value;
    if loaded.recovery.is_some() {
        eprintln!(
            "warning: the remote config was unreadable and has been reset — \
             the login throttle and TOTP replay guard start from zero, and \
             the surface is disabled until reconfigured."
        );
    }
    if !cfg.server.enabled {
        anyhow::bail!("the remote surface is disabled — `claudepot remote enable` first");
    }
    if cfg.password_hash.is_none() {
        anyhow::bail!("no password is set — `claudepot remote set-password` first");
    }

    let devices = device_store::load().context("load devices")?.value;
    let server_cfg = cfg.server.clone();
    let approvals_enabled = cfg.approvals_enabled;
    let state = Arc::new(Mutex::new(AppState {
        config: cfg,
        devices,
        persist: Box::new(FilePersist),
        idempotency: claudepot_core::remote::idempotency::Idempotency::new(),
        challenges: claudepot_core::remote::passkey::Challenges::new(),
    }));

    let (_listener, info) = serve::listen(&server_cfg).await?;
    // `listen` bound a socket purely to report the address; drop it so
    // `serve` can bind the same one. Racy in principle, harmless here:
    // a port stolen in that window surfaces as a plain bind error.
    drop(_listener);

    ctx.info(&format!("Serving {} — Ctrl-C to stop", info.url()));
    if !info.tls {
        ctx.info("Loopback: no certificate needed (already a secure context).");
    }

    // Remote approval is armed for exactly as long as the surface that
    // reaches the phone is up — that coupling is what makes it
    // acceptable to hand a network client the ability to grant a
    // permission at all. Installed here, revoked on the way out, and
    // heartbeated in between so a `kill -9` disarms it too.
    let approvals = ApprovalHook::arm(approvals_enabled);
    // Deliberately not `ctx.info`: `--quiet` must not hide the fact
    // that a feature is not working.
    for w in &approvals.warnings {
        eprintln!("warning: {w}");
    }

    tokio::select! {
        result = serve::serve(&server_cfg, router(state)) => result?,
        // Without this the process dies inside `serve` and the hook
        // entry outlives it in the user's settings.json. The heartbeat
        // makes a survivor harmless, but leaving litter we could have
        // removed is not a plan — and this file is one people read.
        signal = shutdown_signal() => ctx.info(&format!("{signal}. Stopping.")),
    }
    Ok(())
}

/// Resolves when the process is asked to stop.
///
/// SIGTERM as well as Ctrl-C, and the difference is not academic:
/// `kill`, launchd, systemd and every process supervisor send SIGTERM,
/// so handling only SIGINT means the ordinary way to stop a server is
/// the one way that leaves the hook installed. Measured — the first
/// version of this handler did exactly that.
async fn shutdown_signal() -> &'static str {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        // If the handler cannot be installed there is nothing useful to
        // do but wait on the other one; `pending` never resolves.
        let mut term = match signal(SignalKind::terminate()) {
            Ok(s) => s,
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
                return "Interrupted";
            }
        };
        tokio::select! {
            _ = tokio::signal::ctrl_c() => "Interrupted",
            _ = term.recv() => "Terminated",
        }
    }
    #[cfg(not(unix))]
    {
        // Windows has no SIGTERM; `ctrl_c` also covers the console
        // close and shutdown events that reach a foreground process.
        let _ = tokio::signal::ctrl_c().await;
        "Interrupted"
    }
}

#[cfg(test)]
mod tests {
    use super::reach_line;
    use std::net::IpAddr;

    fn line(s: &str) -> String {
        reach_line(&s.parse::<IpAddr>().unwrap())
    }

    #[test]
    fn loopback_reaches_only_this_machine() {
        for a in ["127.0.0.1", "::1"] {
            assert_eq!(line(a), "this machine only", "{a}");
        }
    }

    #[test]
    fn a_tailnet_address_says_from_anywhere() {
        // 100.64.0.0/10 — the range `bind` allows precisely so a mesh
        // VPN works. This is the one case where "from anywhere" is true.
        for a in ["100.64.0.1", "100.96.0.1", "100.127.255.255"] {
            let l = line(a);
            assert!(l.contains("tailnet"), "{a}: {l}");
            assert!(l.contains("anywhere"), "{a}: {l}");
        }
    }

    #[test]
    fn a_lan_address_says_this_network_and_warns_off_port_forwarding() {
        // The default case, and the one nothing in the product stated.
        for a in ["192.168.1.10", "10.0.0.1", "172.16.4.2", "169.254.1.1"] {
            let l = line(a);
            assert!(l.contains("this network only"), "{a}: {l}");
            assert!(l.contains("Tailscale"), "{a}: {l}");
            assert!(
                l.contains("port-forward"),
                "the router path is the one the allowlist cannot close — {a}: {l}"
            );
        }
    }

    #[test]
    fn the_tailscale_range_boundaries_are_not_off_by_one() {
        // 100.63.255.255 and 100.128.0.0 are ordinary public space, so
        // treating them as tailnet would promise reachability the
        // allowlist would in fact have refused outright.
        for a in ["100.63.255.255", "100.128.0.1"] {
            assert!(!line(a).contains("tailnet"), "{a}");
        }
    }
}
