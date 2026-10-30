use std::io::{BufRead as _, IsTerminal as _, Write as _};

use anyhow::{bail, Result};
use claudepot_core::relay::client_ops::{self, ClientCtx};

use crate::output::print_json;
use crate::AppContext;

pub async fn pair_cmd(
    ctx: &AppContext,
    address: &str,
    code: &str,
    fingerprint: Option<&str>,
    name: Option<&str>,
    device_name: Option<&str>,
) -> Result<()> {
    let ops = ClientCtx::production();
    // The user brings the server's fingerprint; we never ask them to
    // confirm ours, which an attacker would also show. Resolved before
    // any network contact, so a non-interactive call without it is
    // refused having touched nothing.
    let approved = match fingerprint {
        Some(f) => f.to_owned(),
        None => {
            if !std::io::stdin().is_terminal() {
                bail!("pass --fingerprint: the server's CA fingerprint, as its pairing screen shows it");
            }
            eprint!("CA fingerprint shown on the relay server: ");
            std::io::stderr().flush()?;
            let mut line = String::new();
            std::io::stdin().lock().read_line(&mut line)?;
            line.trim().to_owned()
        }
    };
    let offer = client_ops::fetch_ca(address).await?;
    let device = device_name
        .map(str::to_owned)
        .unwrap_or_else(client_ops::default_device_name);
    let record = client_ops::complete_pair(&ops, &offer, &approved, code, &device, name).await?;

    if ctx.json {
        print_json(&serde_json::json!({
            "server_id": record.server_id,
            "name": record.name,
            "base_url": record.base_url,
            "default_upstream": record.default_upstream.as_ref().map(|u| u.as_str()),
            "upstreams": record.upstreams.iter().map(|u| u.id.as_str()).collect::<Vec<_>>(),
        }))?;
    } else {
        println!("Paired with {} ({}).", record.name, record.base_url);
        let ups: Vec<&str> = record.upstreams.iter().map(|u| u.id.as_str()).collect();
        if !ups.is_empty() {
            println!("Upstreams: {}", ups.join(", "));
        }
        println!("Run Claude Code through it with: claudepot relay run");
    }
    Ok(())
}
