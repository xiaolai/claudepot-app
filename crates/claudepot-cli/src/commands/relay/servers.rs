use std::io::{BufRead as _, IsTerminal as _, Write as _};

use anyhow::{bail, Result};
use claudepot_core::relay::client_ops::{self, ClientCtx};

use crate::output::print_json;
use crate::AppContext;

pub async fn servers_cmd(ctx: &AppContext) -> Result<()> {
    let views = client_ops::list(&ClientCtx::production(), chrono::Utc::now()).await?;
    if ctx.json {
        let rows: Vec<_> = views
            .iter()
            .map(|v| {
                serde_json::json!({
                    "server_id": v.record.server_id,
                    "name": v.record.name,
                    "base_url": v.record.base_url,
                    "reachable": v.probe.is_ok(),
                    "error": v.probe.as_ref().err().map(|e| e.message.as_str()),
                    "rtt_ms": v.probe.as_ref().ok().map(|p| p.rtt_ms),
                    "default_upstream": v.record.default_upstream.as_ref().map(|u| u.as_str()),
                    "upstreams": v.upstreams.iter().map(|(id, verdict)| serde_json::json!({
                        "id": id.as_str(),
                        "eligible": verdict.is_ok(),
                        "tier": verdict.as_ref().ok().map(|r| r.tier.to_string()),
                        "score_ms": verdict.as_ref().ok().map(|r| r.score_ms),
                        "reason": verdict.as_ref().err().map(|e| e.to_string()),
                    })).collect::<Vec<_>>(),
                })
            })
            .collect();
        return print_json(&rows);
    }
    if views.is_empty() {
        println!("No paired relays. Pair one with: claudepot relay pair <host> --code <code>");
        return Ok(());
    }
    for v in &views {
        match &v.probe {
            Ok(p) => println!("{}  {}  {} ms", v.record.name, v.record.base_url, p.rtt_ms),
            Err(e) => println!("{}  {}  unreachable: {e}", v.record.name, v.record.base_url),
        }
        for (id, verdict) in &v.upstreams {
            let default = v.record.default_upstream.as_ref() == Some(id);
            let mark = if default { " (default)" } else { "" };
            match verdict {
                Ok(r) => println!(
                    "  {}{mark}: ready — {}, {} ms",
                    id.as_str(),
                    r.tier,
                    r.score_ms
                ),
                Err(why) => println!("  {}{mark}: {why}", id.as_str()),
            }
        }
    }
    Ok(())
}

/// Show what `upstream` now serves, and record acceptance of exactly
/// that value once the user agrees.
pub async fn accept_cmd(ctx: &AppContext, server: &str, upstream: Option<&str>) -> Result<()> {
    let ops = ClientCtx::production();
    let upstream = match upstream {
        Some(u) => u.to_owned(),
        None => client_ops::default_upstream_for(&ops, server)?,
    };
    let shown = client_ops::show_acceptance(&ops, server, &upstream).await?;
    // `accepted` covers the server's grant *and* this client's approval;
    // when only one side holds it, the confirmation below repairs the
    // other rather than reporting success `relay run` would contradict.
    if shown.accepted {
        if ctx.json {
            print_json(&shown_json(&shown))?;
        } else {
            println!("{upstream} on {server} is already accepted.");
        }
        return Ok(());
    }
    // On stderr in every mode: `--json` keeps stdout for the result, but
    // the prompt below must never ask to accept a target it has not shown.
    eprintln!(
        "{upstream} on {server} now serves a different target (fingerprint {}, generation {}).",
        shown.fingerprint, shown.generation
    );
    confirm_acceptance(ctx.yes, std::io::stdin().is_terminal(), || {
        eprint!("Accept it? [y/N] ");
        std::io::stderr().flush()?;
        let mut line = String::new();
        std::io::stdin().lock().read_line(&mut line)?;
        Ok(line)
    })?;
    let a = client_ops::record_acceptance(&ops, server, &upstream, &shown.fingerprint).await?;
    if ctx.json {
        print_json(&shown_json(&a))?;
    } else {
        println!("Accepted {upstream} on {server}.");
    }
    Ok(())
}

/// The gate before [`client_ops::record_acceptance`]: `--yes`, or an
/// explicit yes typed at a terminal. Anything else refuses, and the
/// caller records nothing.
fn confirm_acceptance(
    yes: bool,
    interactive: bool,
    ask: impl FnOnce() -> std::io::Result<String>,
) -> Result<()> {
    if yes {
        return Ok(());
    }
    if !interactive {
        bail!("pass --yes to accept non-interactively");
    }
    let line = ask()?;
    if !matches!(line.trim(), "y" | "Y" | "yes") {
        bail!("not accepted");
    }
    Ok(())
}

fn shown_json(a: &claudepot_core::relay::client::Acceptance) -> serde_json::Value {
    serde_json::json!({
        "upstream": a.upstream,
        "fingerprint": a.fingerprint,
        "generation": a.generation,
        "accepted": a.accepted,
    })
}

#[cfg(test)]
mod tests {
    use super::confirm_acceptance;

    #[test]
    fn acceptance_without_yes_off_a_terminal_is_refused_without_asking() {
        let err = confirm_acceptance(false, false, || panic!("must not prompt")).unwrap_err();
        assert!(err.to_string().contains("--yes"), "{err}");
    }

    #[test]
    fn acceptance_needs_an_explicit_yes_at_the_prompt() {
        for answer in ["\n", "n\n", "no\n", "yes please\n"] {
            assert!(
                confirm_acceptance(false, true, || Ok(answer.into())).is_err(),
                "{answer:?} must not accept"
            );
        }
        for answer in ["y\n", "Y\n", "yes\n"] {
            assert!(confirm_acceptance(false, true, || Ok(answer.into())).is_ok());
        }
    }

    #[test]
    fn yes_accepts_without_prompting() {
        assert!(confirm_acceptance(true, false, || panic!("must not prompt")).is_ok());
    }
}
