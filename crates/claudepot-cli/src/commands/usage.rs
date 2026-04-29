//! `claudepot usage` — local-tracking cost report derived from CC
//! transcripts on disk. Mirrors what CC's own `/usage` view computes
//! locally for the "this install" totals: token counts × the bundled
//! pricing table, rolled up by project.
//!
//! No network call. No account attribution (claudepot doesn't keep a
//! swap-event log; per-account "who paid" requires infrastructure
//! we haven't built yet — see `claudepot_core::usage_local` docs for
//! the rationale and the deferred path).
//!
//! Exit code is always 0 on success — even when the index is empty.
//! `--json` switches the output to a single object that scripting
//! pipes can parse directly.

use crate::AppContext;
use anyhow::{Context, Result};
use chrono::{DateTime, TimeZone, Utc};
use claudepot_core::pricing;
use claudepot_core::session::list_all_sessions;
use claudepot_core::usage_local::{
    aggregate_from_rows, LocalUsageReport, ProjectUsageRow, TimeWindow,
};

/// Parse the `--window` flag value. Accepted forms:
///
/// - `all` (default) — open-ended on both sides.
/// - `7d`, `30d`, `90d` — last N days, anchored at "now".
/// - `<n>d` — same shape, any positive integer.
///
/// Anything else is rejected with a clap-style error so the CLI
/// surface stays predictable.
fn parse_window(raw: &str, now_ms: i64) -> Result<TimeWindow> {
    if raw == "all" {
        return Ok(TimeWindow::open());
    }
    let bytes = raw.as_bytes();
    if bytes.len() < 2 || bytes[bytes.len() - 1] != b'd' {
        anyhow::bail!(
            "window must be `all` or `<n>d` (e.g. `7d`, `30d`); got `{raw}`"
        );
    }
    let n: u32 = raw[..raw.len() - 1]
        .parse()
        .with_context(|| format!("window: not a number: `{raw}`"))?;
    Ok(TimeWindow::last_days(n, now_ms))
}

pub async fn report(ctx: &AppContext, window: &str) -> Result<()> {
    let now_ms = Utc::now().timestamp_millis();
    let tw = parse_window(window, now_ms)?;

    let config_dir = claudepot_core::paths::claude_config_dir();
    let sessions = list_all_sessions(&config_dir)
        .with_context(|| "failed to read CC session index")?;

    // Pricing comes from the bundled defaults. The cache-and-refresh
    // service exists for the GUI; CLI is one-shot, so paying the
    // bundled-rate path is the right tradeoff.
    let prices = pricing::load();

    let report = aggregate_from_rows(sessions, &prices, tw);

    if ctx.json {
        print_json(&report)?;
    } else {
        print_human(&report);
    }
    Ok(())
}

fn print_json(r: &LocalUsageReport) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(r)?);
    Ok(())
}

fn print_human(r: &LocalUsageReport) {
    let header = window_header(r);
    println!("{header}");
    println!();
    if r.rows.is_empty() {
        println!("No sessions in window.");
        return;
    }

    // Column widths chosen to fit a typical 100-col terminal. Project
    // path takes the residual slack so long paths don't wrap.
    println!(
        "{:<10}  {:>5}  {:>13}  {:>13}  {:>13}  {:>15}  {:>11}  {}",
        "LAST", "SESS", "INPUT", "OUTPUT", "C-WRITE", "C-READ", "COST USD", "PROJECT"
    );
    println!("{}", "-".repeat(110));
    for row in &r.rows {
        print_row(row);
    }
    println!("{}", "-".repeat(110));
    print_totals(r);
    if r.totals.unpriced_sessions > 0 {
        println!();
        println!(
            "note: {} session(s) had no priced model — token counts above include them; cost excludes them.",
            r.totals.unpriced_sessions
        );
    }
}

fn window_header(r: &LocalUsageReport) -> String {
    let from = r
        .window
        .from_ms
        .map(format_ms_short)
        .unwrap_or_else(|| "—".to_string());
    let to = r
        .window
        .to_ms
        .map(format_ms_short)
        .unwrap_or_else(|| "—".to_string());
    format!("Window: {from} → {to}")
}

fn format_ms_short(ms: i64) -> String {
    Utc.timestamp_millis_opt(ms)
        .single()
        .map(|t: DateTime<Utc>| t.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| "?".to_string())
}

fn print_row(row: &ProjectUsageRow) {
    let last = row
        .last_active_ms
        .map(format_ms_short)
        .unwrap_or_else(|| "—".to_string());
    let cost = row
        .cost_usd
        .map(|c| format!("${c:.2}"))
        .unwrap_or_else(|| "n/a".to_string());
    println!(
        "{:<10}  {:>5}  {:>13}  {:>13}  {:>13}  {:>15}  {:>11}  {}",
        last,
        row.session_count,
        thousands(row.tokens_input),
        thousands(row.tokens_output),
        thousands(row.tokens_cache_creation),
        thousands(row.tokens_cache_read),
        cost,
        row.project_path,
    );
}

fn print_totals(r: &LocalUsageReport) {
    let cost = r
        .totals
        .cost_usd
        .map(|c| format!("${c:.2}"))
        .unwrap_or_else(|| "n/a".to_string());
    println!(
        "{:<10}  {:>5}  {:>13}  {:>13}  {:>13}  {:>15}  {:>11}  {}",
        "TOTAL",
        r.totals.session_count,
        thousands(r.totals.tokens_input),
        thousands(r.totals.tokens_output),
        thousands(r.totals.tokens_cache_creation),
        thousands(r.totals.tokens_cache_read),
        cost,
        format!(
            "({} project{})",
            r.rows.len(),
            if r.rows.len() == 1 { "" } else { "s" }
        ),
    );
}

/// Render `12345678` as `12,345,678`. Pure formatting — kept inline
/// because the CLI's only thousands-grouping consumer is this column
/// pack and pulling in a humanise crate for one helper would be churn.
fn thousands(n: u64) -> String {
    let s = n.to_string();
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    let len = bytes.len();
    for (i, b) in bytes.iter().enumerate() {
        if i > 0 && (len - i) % 3 == 0 {
            out.push(',');
        }
        out.push(*b as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_window_all_is_open() {
        let w = parse_window("all", 1_700_000_000_000).unwrap();
        assert!(w.from_ms.is_none() && w.to_ms.is_none());
    }

    #[test]
    fn parse_window_seven_days_anchors() {
        let now = 1_700_000_000_000;
        let w = parse_window("7d", now).unwrap();
        assert_eq!(w.to_ms, Some(now));
        assert_eq!(w.from_ms, Some(now - 7 * 86_400_000));
    }

    #[test]
    fn parse_window_zero_days_is_open() {
        // `0d` yields an open window via TimeWindow::last_days(0, _).
        let w = parse_window("0d", 1_700_000_000_000).unwrap();
        assert!(w.from_ms.is_none() && w.to_ms.is_none());
    }

    #[test]
    fn parse_window_rejects_garbage() {
        assert!(parse_window("yesterday", 0).is_err());
        assert!(parse_window("7", 0).is_err());
        assert!(parse_window("d", 0).is_err());
        assert!(parse_window("", 0).is_err());
    }

    #[test]
    fn thousands_groups_by_three() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1_000), "1,000");
        assert_eq!(thousands(1_234_567), "1,234,567");
        assert_eq!(thousands(12_345_678_901), "12,345,678,901");
    }
}
