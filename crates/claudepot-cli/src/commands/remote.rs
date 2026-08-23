//! `claudepot remote …` — the LAN appliance surface.
//!
//! Thin over `claudepot_core::remote`; every decision (bind allowlist,
//! TLS requirement, password hashing, throttle) belongs to core.
//!
//! Passwords are read from **stdin**, following the convention
//! `account add` already set — this CLI deliberately does not ship
//! `rpassword`. Typing one interactively echoes it, so the piped form
//! is the documented one.

use std::sync::Arc;

use anyhow::{bail, Context, Result};
use claudepot_core::remote::config::{self, RemoteConfigFile};
use claudepot_core::remote::server::{router, AppState, Persist};
use claudepot_core::remote::{approval, password, serve, store as device_store, DevicesFile};
use tokio::sync::Mutex;

use crate::output::print_json;
use crate::AppContext;

/// Writes both files the surface owns.
struct FilePersist;

impl Persist for FilePersist {
    fn save(&self, config: &RemoteConfigFile, devices: &DevicesFile) -> std::io::Result<()> {
        config::save(config).map_err(|e| std::io::Error::other(format!("{e:?}")))?;
        device_store::save(devices).map_err(|e| std::io::Error::other(format!("{e:?}")))
    }
}

fn load_config() -> Result<RemoteConfigFile> {
    let loaded = config::load().context("load remote config")?;
    if loaded.recovery.is_some() {
        // Never silent: the throttle counter and the spent-TOTP
        // high-water mark were in that file.
        eprintln!(
            "warning: the remote config was unreadable and has been reset — \
             the login throttle and TOTP replay guard start from zero, and \
             the surface is disabled until reconfigured."
        );
    }
    Ok(loaded.value)
}

pub fn status_cmd(ctx: &AppContext) -> Result<()> {
    let cfg = load_config()?;
    let bind = cfg.server.checked_bind();

    if ctx.json {
        print_json(&serde_json::json!({
            "enabled": cfg.server.enabled,
            "bind": cfg.server.bind.to_string(),
            "port": cfg.server.port,
            "password_set": cfg.password_hash.is_some(),
            "totp_enabled": cfg.totp_secret_base32.is_some(),
            "requires_tls": bind.as_ref().map(|b| b.requires_tls()).unwrap_or(true),
            "bind_error": bind.as_ref().err().map(|e| e.to_string()),
        }))?;
        return Ok(());
    }

    println!(
        "Remote surface: {}",
        if cfg.server.enabled { "ON" } else { "off" }
    );
    println!("  bind      {}:{}", cfg.server.bind, cfg.server.port);
    match &bind {
        Ok(b) => println!(
            "  tls       {}",
            if b.requires_tls() {
                "required (not loopback)"
            } else {
                "not needed (loopback is already a secure context)"
            }
        ),
        Err(e) => println!("  bind      REFUSED: {e}"),
    }
    println!(
        "  password  {}",
        if cfg.password_hash.is_some() {
            "set"
        } else {
            "NOT SET — the surface will refuse every login"
        }
    );
    println!(
        "  2FA       {}",
        if cfg.totp_secret_base32.is_some() {
            "on"
        } else {
            "off"
        }
    );
    Ok(())
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

    if plain.is_empty() {
        plain.zeroize();
        bail!("empty password");
    }
    // `hash_password` zeroizes `plain` on every path, success or error.
    let hash = password::hash_password(&mut plain)?;

    let mut cfg = load_config()?;
    cfg.password_hash = Some(hash);
    // A new password ends the lockout: the person who can set it is
    // the owner, and leaving them throttled would be theatre.
    cfg.failed_attempts = 0;
    config::save(&cfg).map_err(|e| anyhow::anyhow!("{e:?}"))?;

    if ctx.json {
        print_json(&serde_json::json!({ "password_set": true }))?;
    } else {
        println!("Password set. Existing sessions are unaffected —");
        println!("revoke them with `claudepot remote revoke-all` if that is not what you want.");
    }
    Ok(())
}

pub fn enable_cmd(ctx: &AppContext, bind: Option<&str>, port: Option<u16>) -> Result<()> {
    let mut cfg = load_config()?;
    if let Some(b) = bind {
        cfg.server.bind = b
            .parse()
            .with_context(|| format!("not an IP address: {b}"))?;
    }
    if let Some(p) = port {
        cfg.server.port = p;
    }
    // Refuse before writing, so a bad address never reaches disk.
    let checked = cfg.server.checked_bind()?;
    if cfg.password_hash.is_none() {
        bail!("set a password first (`claudepot remote set-password`) — enabling without one would open a surface nobody can log into, and that is not a useful state to persist");
    }
    cfg.server.enabled = true;
    config::save(&cfg).map_err(|e| anyhow::anyhow!("{e:?}"))?;

    if ctx.json {
        print_json(&serde_json::json!({
            "enabled": true,
            "bind": cfg.server.bind.to_string(),
            "port": cfg.server.port,
            "requires_tls": checked.requires_tls(),
        }))?;
        return Ok(());
    }
    println!(
        "Remote surface enabled on {}:{}.",
        cfg.server.bind, cfg.server.port
    );
    if checked.requires_tls() {
        println!("TLS is required for this address. Mint a certificate first:");
        println!("  ./scripts/mint-remote-cert.sh <hostname>");
    }
    println!("Start it with `claudepot remote serve`.");
    Ok(())
}

pub fn disable_cmd(ctx: &AppContext) -> Result<()> {
    let mut cfg = load_config()?;
    cfg.server.enabled = false;
    config::save(&cfg).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    if ctx.json {
        print_json(&serde_json::json!({ "enabled": false }))?;
    } else {
        println!("Remote surface disabled.");
    }
    Ok(())
}

pub fn revoke_all_cmd(ctx: &AppContext) -> Result<()> {
    let loaded = device_store::load().context("load devices")?;
    let mut devices = loaded.value;
    let now = chrono::Utc::now();
    let mut n = 0;
    for d in devices.devices.iter_mut() {
        if d.revoked_at.is_none() {
            // Set, never delete — a deleted device is merely unknown,
            // a revoked one is refused.
            d.revoked_at = Some(now);
            n += 1;
        }
    }
    device_store::save(&devices).map_err(|e| anyhow::anyhow!("{e:?}"))?;
    if ctx.json {
        print_json(&serde_json::json!({ "revoked": n }))?;
    } else {
        println!("Revoked {n} session(s)/device(s).");
    }
    Ok(())
}

pub async fn serve_cmd(ctx: &AppContext) -> Result<()> {
    let cfg = load_config()?;
    if !cfg.server.enabled {
        bail!("the remote surface is disabled — `claudepot remote enable` first");
    }
    if cfg.password_hash.is_none() {
        bail!("no password is set — `claudepot remote set-password` first");
    }

    let devices = device_store::load().context("load devices")?.value;
    let server_cfg = cfg.server.clone();
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
    let _approvals = ApprovalHook::arm();

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

/// Installs CC's `PermissionRequest` hook for the life of the server,
/// and takes it back out on the way down.
struct ApprovalHook {
    beat: tokio::task::JoinHandle<()>,
    installed: bool,
}

impl ApprovalHook {
    fn arm() -> Self {
        let dir = approval::store::dir();
        // Heartbeat first: it is what the hook actually believes, so a
        // hook that starts before the first beat must read "not
        // serving" rather than wait on a server that is not up yet.
        let _ = approval::store::mark_serving(&dir, approval::now_ms());

        // `current_exe` and not a looked-up path: this binary is the
        // one carrying the verb, so the hook cannot point at a
        // Claudepot that is not this one.
        let installed = match std::env::current_exe() {
            Ok(binary) => match approval::install::install(&binary) {
                Ok(_) => true,
                Err(e) => {
                    // Not fatal — the rest of the panel works. But it
                    // must be said: the alternative is a phone that
                    // silently never shows an approval card.
                    // Deliberately not `ctx.info`: `--quiet` must not
                    // hide the fact that a feature is not working.
                    eprintln!(
                        "warning: remote approval is OFF — could not install Claude \
                         Code's PermissionRequest hook: {e}"
                    );
                    false
                }
            },
            Err(e) => {
                eprintln!("warning: remote approval is OFF — cannot locate this binary: {e}");
                false
            }
        };

        let beat = tokio::spawn(async move {
            let mut tick = tokio::time::interval(approval::HEARTBEAT);
            loop {
                tick.tick().await;
                let now = approval::now_ms();
                let _ = approval::store::mark_serving(&dir, now);
                // Cheap, and the only thing that clears up after a hook
                // that was killed with its session.
                approval::store::sweep(&dir, now);
            }
        });

        Self { beat, installed }
    }
}

impl Drop for ApprovalHook {
    fn drop(&mut self) {
        self.beat.abort();
        approval::store::stop_serving(&approval::store::dir());
        if self.installed {
            // Best effort: the runtime gate is what makes a survivor
            // harmless, so a failure here is untidy rather than unsafe.
            let _ = approval::install::uninstall();
        }
    }
}
