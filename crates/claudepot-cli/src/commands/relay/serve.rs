use anyhow::Result;
use claudepot_core::relay::revoke_socket::{self, LiveRevoke, Unavailable};
use claudepot_core::relay::runtime::Reset;
use claudepot_core::relay::service;

use crate::output::print_json;
use crate::AppContext;

/// Serve this machine's relay in the foreground until Ctrl-C / SIGTERM.
///
/// While it serves, this process owns the device list, and `relay revoke`
/// from another shell revokes through it, over its revoke socket. Without
/// one — Windows, or the warning printed at start — stop the server to
/// revoke.
pub async fn serve_cmd(ctx: &AppContext, pair: bool, include_future: bool) -> Result<()> {
    let accounts = ctx.store.list()?;
    let dir = service::default_dir();
    let mut served = service::serve(&dir, &accounts).await?;

    // Not `ctx.info`: `--quiet` must not hide that something is broken.
    for r in &served.resets {
        let what = match r {
            Reset::Config => {
                "the relay config was unreadable and has been reset — every \
                 device must accept its upstreams again"
            }
            Reset::Devices => {
                "the relay device file was unreadable and has been reset — \
                 every device must pair again"
            }
        };
        eprintln!("warning: {what}");
    }
    for w in &served.warnings {
        eprintln!(
            "warning: upstream {} is not ready: {}",
            w.upstream.as_str(),
            w.code
        );
    }
    for id in &served.republished {
        eprintln!(
            "note: the account behind upstream {} changed; devices must accept it again",
            id.as_str()
        );
    }
    match served.running.live_revoke {
        LiveRevoke::Listening => {}
        // A platform limit rather than a fault, and `relay revoke` says so
        // again when it is refused.
        LiveRevoke::Unavailable(Unavailable::Unsupported) => ctx.info(
            "note: on this platform `relay revoke` cannot reach a serving relay; \
             stop it to revoke a device",
        ),
        LiveRevoke::Unavailable(why) => eprintln!(
            "warning: `relay revoke` from another shell cannot reach this relay: {why} ({}); \
             stop the relay to revoke a device",
            revoke_socket::socket_path(&dir).display()
        ),
    }

    ctx.info(&format!(
        "Relay serving on https://{} — Ctrl-C to stop",
        served.running.addr
    ));
    if let Some(fp) = &served.ca_fingerprint {
        ctx.info(&format!("CA fingerprint: {fp}"));
    }
    let offer = if pair {
        Some(service::pair_code(&served.running.state, include_future)?)
    } else {
        None
    };
    if ctx.json {
        // One object, printed once the listener is up; the process then
        // serves until signalled, with progress on stderr as always.
        print_json(&serde_json::json!({
            "addr": served.running.addr.to_string(),
            "ca_fingerprint": served.ca_fingerprint,
            "pairing": offer.as_ref().map(|o| serde_json::json!({
                "code": o.code,
                "ca_fingerprint": o.ca_fingerprint,
                "expires_in_secs": o.expires_in.as_secs(),
            })),
        }))?;
    } else if let Some(offer) = &offer {
        // The code is the point of `--pair`: printed even under --quiet.
        println!("Pairing code: {}", offer.code);
        println!("Fingerprint:  {}", offer.ca_fingerprint);
        println!(
            "Valid for {} minutes. On the client: claudepot relay pair <this-host> --code {}",
            offer.expires_in.as_secs() / 60,
            offer.code
        );
    }

    let why = crate::commands::remote::shutdown_signal().await;
    // Wait for open connections to finish or hit the drain bound, then let
    // go of the relay before saying "stopped": `relay-serve.lock` goes with
    // the last reference to its state, and once the drain has returned this
    // is the last one.
    served.running.stop_and_drain().await;
    drop(served);
    ctx.info(&format!("{why} — relay stopped"));
    Ok(())
}
