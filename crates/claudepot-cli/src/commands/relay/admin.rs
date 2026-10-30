//! Server-side relay administration: what this machine serves and to
//! whom. Every decision is in [`claudepot_core::relay::service`].

use anyhow::{bail, Result};
use claudepot_core::relay::devices::Revocation;
use claudepot_core::relay::service::{self, DeviceSummary, PublicationView, ServerStatus, Via};
use claudepot_core::relay::target::UpstreamId;
use claudepot_core::relay::translate::Translation;
use claudepot_core::remote::service::exposure_word;

use crate::output::print_json;
use crate::AppContext;

fn upstream(id: &str) -> Result<UpstreamId> {
    match UpstreamId::parse(id) {
        Some(u) => Ok(u),
        None => bail!("not a valid upstream id: {id:?}"),
    }
}

/// `usage_unavailable`: the ledger could not be read, so `last_seen` and
/// `today` are unknown — emitted as `null`, never as zeros or "never".
fn device_json(d: &DeviceSummary, usage_unavailable: bool) -> serde_json::Value {
    let (last_seen, today) = if usage_unavailable {
        (serde_json::Value::Null, serde_json::Value::Null)
    } else {
        (
            serde_json::json!(d.last_seen),
            serde_json::json!({
                "requests": d.today.requests,
                "failed": d.today.failed,
                "cost_usd_estimate": d.today.cost_usd,
                "unpriced": d.today.unpriced,
                "family_estimate": d.today.family_estimate,
            }),
        )
    };
    serde_json::json!({
        "id": d.id,
        "name": d.name,
        "created_at": d.created_at,
        "revoked_at": d.revoked_at,
        "include_future": d.include_future,
        "grants": d.grants.iter().map(|g| serde_json::json!({
            "upstream": g.upstream_id.as_str(),
            "accepted_generation": g.accepted_generation,
        })).collect::<Vec<_>>(),
        "usage_unavailable": usage_unavailable,
        "last_seen": last_seen,
        "today": today,
    })
}

fn status_json(s: &ServerStatus) -> serde_json::Value {
    serde_json::json!({
        "server_id": s.server_id,
        "bind": s.bind.to_string(),
        "port": s.port,
        "names": s.names,
        "exposure": s.exposure.map(exposure_word),
        "bind_error": s.bind_error,
        "serving": s.serving,
        "ca_fingerprint": s.ca_fingerprint,
        "config_recovered": s.config_recovered,
        "devices_recovered": s.devices_recovered,
        "usage_unavailable": s.usage_unavailable,
        "publications": s.publications.iter().map(|p| serde_json::json!({
            "id": p.id.as_str(),
            "generation": p.generation,
            "account": p.account_email,
            "route_id": p.route_id,
            "route": p.route_name,
        })).collect::<Vec<_>>(),
        "devices": s.devices.iter().map(|d| device_json(d, s.usage_unavailable)).collect::<Vec<_>>(),
    })
}

fn load(ctx: &AppContext) -> Result<ServerStatus> {
    let accounts = ctx.store.list()?;
    Ok(service::status(
        &service::default_dir(),
        &accounts,
        chrono::Utc::now(),
    )?)
}

/// `≈$0.12`, or nothing when no request was priced.
fn cost(d: &DeviceSummary) -> String {
    if d.today.requests == d.today.unpriced {
        return String::new();
    }
    format!(", ≈${:.2}", d.today.cost_usd)
}

/// One `relay devices` line. With the ledger unreadable, history is
/// "usage unknown", not "never seen" with no requests.
fn device_line(d: &DeviceSummary, usage_unavailable: bool) -> String {
    let state = if d.revoked_at.is_some() {
        " (revoked)"
    } else {
        ""
    };
    let history = if usage_unavailable {
        "usage unknown".to_owned()
    } else {
        let seen = d
            .last_seen
            .map(|t| format!("last seen {}", t.format("%Y-%m-%d %H:%M UTC")))
            .unwrap_or_else(|| "never seen".into());
        let today = if d.today.requests > 0 {
            format!(", {} requests today{}", d.today.requests, cost(d))
        } else {
            String::new()
        };
        format!("{seen}{today}")
    };
    format!("  {}  {}{state} — {history}", d.id, d.name)
}

/// The stderr warnings for stores `status` found reset or unreadable.
fn print_store_warnings(s: &ServerStatus) {
    if s.config_recovered {
        eprintln!("warning: the relay config was unreadable and has been reset");
    }
    if s.devices_recovered {
        eprintln!(
            "warning: the relay device file was unreadable and has been reset — \
             earlier revocations are lost"
        );
    }
    if s.usage_unavailable {
        eprintln!("warning: the relay usage ledger could not be read — today's usage is unknown");
    }
}

fn listen_line(s: &ServerStatus) -> String {
    match (s.exposure, &s.bind_error) {
        (Some(e), _) => format!("Listen: {}:{} ({})", s.bind, s.port, exposure_word(e)),
        (None, Some(err)) => format!("Listen: {}:{} — refused: {err}", s.bind, s.port),
        (None, None) => format!("Listen: {}:{}", s.bind, s.port),
    }
}

/// Who a publication serves: its account, its route, or why neither
/// resolves any more.
fn publication_owner(p: &PublicationView) -> &str {
    match (&p.account_email, p.route_id, &p.route_name) {
        (Some(email), _, _) => email.as_str(),
        (None, Some(_), Some(name)) => name.as_str(),
        (None, Some(_), None) => "(route no longer configured)",
        (None, None, _) => "(account no longer registered)",
    }
}

fn print_publications(s: &ServerStatus) {
    if s.publications.is_empty() {
        println!("Publishing nothing. Publish an account with: claudepot relay publish <email>");
        return;
    }
    println!("Publishing:");
    for p in &s.publications {
        println!(
            "  {}  {}  (generation {})",
            p.id.as_str(),
            publication_owner(p),
            p.generation
        );
    }
}

pub fn status_cmd(ctx: &AppContext) -> Result<()> {
    let s = load(ctx)?;
    if ctx.json {
        return print_json(&status_json(&s));
    }
    print_store_warnings(&s);
    let state = if s.serving { "serving" } else { "stopped" };
    println!("Relay: {state}");
    println!("{}", listen_line(&s));
    if let Some(fp) = &s.ca_fingerprint {
        println!("CA fingerprint: {fp}");
    }
    print_publications(&s);
    let live = s.devices.iter().filter(|d| d.revoked_at.is_none()).count();
    if live > 0 {
        println!("Devices: {live} paired — `claudepot relay devices` lists them");
    }
    Ok(())
}

pub fn devices_cmd(ctx: &AppContext) -> Result<()> {
    let s = load(ctx)?;
    if ctx.json {
        return print_json(
            &s.devices
                .iter()
                .map(|d| device_json(d, s.usage_unavailable))
                .collect::<Vec<_>>(),
        );
    }
    if s.usage_unavailable {
        eprintln!("warning: the relay usage ledger could not be read — today's usage is unknown");
    }
    if s.devices.is_empty() {
        println!("No paired devices. Pair one with: claudepot relay serve --pair");
        return Ok(());
    }
    for d in &s.devices {
        println!("{}", device_line(d, s.usage_unavailable));
    }
    Ok(())
}

pub fn bind_cmd(ctx: &AppContext, address: &str, port: Option<u16>) -> Result<()> {
    let exposure = service::set_bind(&service::default_dir(), address, port, None)?;
    if ctx.json {
        return print_json(&serde_json::json!({ "exposure": exposure_word(exposure) }));
    }
    let word = exposure_word(exposure);
    println!("Relay will listen on {address} ({word}) the next time it serves.");
    if word == "every_interface" {
        eprintln!(
            "warning: 0.0.0.0 listens on every interface, including any public \
             address this machine acquires later"
        );
    }
    Ok(())
}

pub async fn publish_cmd(ctx: &AppContext, email: &str, id: Option<&str>) -> Result<()> {
    let email = claudepot_core::resolve::resolve_email(&ctx.store, email)?;
    let Some(account) = ctx.store.find_by_email(&email)? else {
        bail!("no registered account {email}");
    };
    let id = id.map(upstream).transpose()?;
    let (id, generation) = service::publish(&service::default_dir(), &account, id).await?;
    if ctx.json {
        return print_json(&serde_json::json!({
            "id": id.as_str(), "account": email, "generation": generation,
        }));
    }
    println!(
        "Publishing {email} as upstream {} (generation {generation}).",
        id.as_str()
    );
    Ok(())
}

pub async fn publish_route_cmd(
    ctx: &AppContext,
    route: &str,
    id: Option<&str>,
    translate: Option<&str>,
) -> Result<()> {
    let translate = translate.map(translation).transpose()?;
    let dir = service::default_dir();
    let routes = service::load_routes(&dir)?;
    let route = service::find_route(&routes, route)?;
    let id = id.map(upstream).transpose()?;
    let (id, generation) = service::publish_route(&dir, route, id, translate).await?;
    let protocol = translate.map(Translation::as_str);
    if ctx.json {
        return print_json(&serde_json::json!({
            "id": id.as_str(), "route": route.name, "route_id": route.id,
            "generation": generation, "translate": protocol,
        }));
    }
    let via = protocol.map_or(String::new(), |p| format!(", translated to {p}"));
    println!(
        "Publishing route {} as upstream {}{via} (generation {generation}).",
        route.name,
        id.as_str()
    );
    Ok(())
}

fn translation(s: &str) -> Result<Translation> {
    Translation::parse(s).ok_or_else(|| {
        let names: Vec<&str> = Translation::ALL.iter().map(|t| t.as_str()).collect();
        anyhow::anyhow!(
            "unknown translation {s:?}; expected one of: {}",
            names.join(", ")
        )
    })
}

pub fn unpublish_cmd(ctx: &AppContext, id: &str) -> Result<()> {
    service::unpublish(&service::default_dir(), &upstream(id)?)?;
    if ctx.json {
        return print_json(&serde_json::json!({ "unpublished": id }));
    }
    ctx.info(&format!("Upstream {id} is no longer published."));
    Ok(())
}

pub fn rotate_ca_cmd(ctx: &AppContext) -> Result<()> {
    if !ctx.yes {
        bail!(
            "refusing to rotate the relay CA without --yes: every paired device is revoked \
             and must pair again"
        );
    }
    let r = service::rotate_ca(&service::default_dir(), std::time::SystemTime::now())?;
    if ctx.json {
        print_json(&serde_json::json!({
            "ca_fingerprint": r.ca_fingerprint,
            "devices_revoked": r.devices_revoked,
        }))?;
    } else {
        println!("New relay CA: {}", r.ca_fingerprint);
        println!(
            "Revoked {} device(s). Every client must pair again.",
            r.devices_revoked
        );
    }
    Ok(())
}

/// Works from any shell: while this machine's relay serves, the revocation
/// goes through it, so the device is refused at once.
pub async fn revoke_cmd(ctx: &AppContext, device_id: &str) -> Result<()> {
    let done = service::revoke_anywhere(&service::default_dir(), device_id).await?;
    let already = done.revocation == Revocation::AlreadyRevoked;
    if ctx.json {
        let via = match done.via {
            Via::Offline => "offline",
            Via::ServingRelay => "serving_relay",
        };
        return print_json(&serde_json::json!({
            "revoked": device_id,
            "via": via,
            "already_revoked": already,
        }));
    }
    ctx.info(&match (already, done.via) {
        (true, _) => format!("Device {device_id} was already revoked."),
        (false, Via::Offline) => format!("Device {device_id} revoked."),
        (false, Via::ServingRelay) => format!(
            "Device {device_id} revoked. The serving relay ended its open streams and \
             refuses its requests from now on."
        ),
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(seen: bool) -> DeviceSummary {
        DeviceSummary {
            id: "dev-1".into(),
            name: "laptop".into(),
            created_at: chrono::Utc::now(),
            revoked_at: None,
            include_future: false,
            grants: Vec::new(),
            last_seen: seen.then(chrono::Utc::now),
            today: Default::default(),
        }
    }

    #[test]
    fn an_unreadable_ledger_is_unknown_usage_in_json_not_zeros() {
        let v = device_json(&device(false), true);
        assert_eq!(v["usage_unavailable"], true);
        assert!(v["last_seen"].is_null());
        assert!(v["today"].is_null(), "unknown usage must not read as zero");

        let v = device_json(&device(false), false);
        assert_eq!(v["usage_unavailable"], false);
        assert_eq!(v["today"]["requests"], 0);
    }

    #[test]
    fn an_unreadable_ledger_is_not_rendered_as_never_seen() {
        let line = device_line(&device(false), true);
        assert!(line.contains("usage unknown"), "{line}");
        assert!(!line.contains("never seen"), "{line}");
        assert!(device_line(&device(false), false).contains("never seen"));
        assert!(device_line(&device(true), false).contains("last seen"));
    }
}
