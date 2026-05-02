//! `claudepot activity` — query and rebuild the activity card index.
//!
//! Two verbs ship in Phase 1:
//!   - `recent` — read the index and print cards, newest first.
//!   - `reindex` — full backfill from `~/.claude/projects/*/*.jsonl`.
//!
//! `tail` (live follow) is Phase 2 — depends on the LiveRuntime
//! integration that v1 deliberately defers.
//!
//! All handlers are thin wrappers around `claudepot_core::activity`.
//! No business logic lives here (per `.claude/rules/architecture.md`
//! and `.claude/rules/commands.md`).

use crate::AppContext;
use anyhow::{Context, Result};
use chrono::{DateTime, Local, Utc};
use claudepot_core::activity::{
    backfill, render_help, ActivityIndex, Card, CardKind, RecentQuery, Severity,
};
use claudepot_core::paths;
use std::path::PathBuf;
use std::time::Duration;

/// Open (or create) the activity index at the canonical location.
/// Same `sessions.db` file as `SessionIndex`; SQLite WAL mode lets
/// the two handles coexist.
fn open_index() -> Result<ActivityIndex> {
    let db_path = paths::claudepot_data_dir().join("sessions.db");
    ActivityIndex::open(&db_path).with_context(|| format!("open {}", db_path.display()))
}

/// `claudepot activity recent` — print recent cards.
#[allow(clippy::too_many_arguments)]
pub fn recent(
    ctx: &AppContext,
    since: Option<&str>,
    kinds: &[String],
    severity: Option<&str>,
    project: Option<&str>,
    plugin: Option<&str>,
    limit: Option<usize>,
) -> Result<()> {
    let idx = open_index()?;
    let mut q = RecentQuery {
        limit,
        ..Default::default()
    };
    if let Some(s) = since {
        q.since_ms = Some(parse_since(s)?);
    }
    if !kinds.is_empty() {
        q.kinds = kinds
            .iter()
            .map(|s| parse_kind(s))
            .collect::<Result<Vec<_>>>()?;
    }
    if let Some(s) = severity {
        q.min_severity = Some(parse_severity(s)?);
    }
    if let Some(p) = project {
        q.project_path_prefix = Some(PathBuf::from(p));
    }
    if let Some(p) = plugin {
        q.plugin = Some(p.to_string());
    }
    let cards = idx.recent(&q).context("query recent activity cards")?;

    if ctx.json {
        let s = serde_json::to_string_pretty(&cards).context("serialize cards")?;
        println!("{s}");
        return Ok(());
    }

    if cards.is_empty() {
        if !ctx.quiet {
            eprintln!("No activity cards. Run `claudepot activity reindex` to populate.");
        }
        return Ok(());
    }
    print_human(&cards);
    Ok(())
}

/// `claudepot activity reindex` — full rebuild of the activity index.
pub fn reindex(ctx: &AppContext) -> Result<()> {
    let idx = open_index()?;
    let config_dir = paths::claude_config_dir();
    if !ctx.quiet {
        eprintln!("Scanning {}…", config_dir.display());
    }
    let stats = backfill::run(&config_dir, &idx).context("backfill")?;

    if ctx.json {
        // Stable shape for downstream tooling: counts and a list of
        // failures (path + error message). The `elapsed` is wall-clock
        // ms — easy to pipe into perf dashboards.
        println!(
            "{}",
            serde_json::json!({
                "files_scanned": stats.files_scanned,
                "cards_inserted": stats.cards_inserted,
                "cards_skipped_duplicates": stats.cards_skipped_duplicates,
                "cards_pruned": stats.cards_pruned,
                "failed": stats.failed.iter().map(|(p, e)| {
                    serde_json::json!({"path": p.display().to_string(), "error": e})
                }).collect::<Vec<_>>(),
                "elapsed_ms": stats.elapsed.as_millis(),
            })
        );
    } else if !ctx.quiet {
        let pruned_note = if stats.cards_pruned > 0 {
            format!(", {} pruned (source JSONL gone)", stats.cards_pruned)
        } else {
            String::new()
        };
        println!(
            "Scanned {} files in {} ms — {} new card(s), {} duplicate(s){}{}.",
            stats.files_scanned,
            stats.elapsed.as_millis(),
            stats.cards_inserted,
            stats.cards_skipped_duplicates,
            pruned_note,
            if stats.failed.is_empty() {
                String::new()
            } else {
                format!(", {} unreadable", stats.failed.len())
            }
        );
        for (path, err) in stats.failed.iter().take(10) {
            eprintln!("  failed: {} — {}", path.display(), err);
        }
        if stats.failed.len() > 10 {
            eprintln!("  …and {} more", stats.failed.len() - 10);
        }
    }
    Ok(())
}

/// Parse `--since` shorthand: `1h`, `30m`, `2d`. Returns absolute
/// ms-since-epoch suitable for `RecentQuery.since_ms`.
fn parse_since(s: &str) -> Result<i64> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        anyhow::bail!("--since requires a value (e.g. 1h, 30m, 2d)");
    }
    let (num_str, unit) = trimmed.split_at(trimmed.len() - 1);
    let n: u64 = num_str
        .parse()
        .with_context(|| format!("invalid --since value: {trimmed}"))?;
    let dur = match unit {
        "s" => Duration::from_secs(n),
        "m" => Duration::from_secs(n * 60),
        "h" => Duration::from_secs(n * 60 * 60),
        "d" => Duration::from_secs(n * 60 * 60 * 24),
        _ => anyhow::bail!("--since unit must be s/m/h/d (got {unit:?})"),
    };
    let now = Utc::now().timestamp_millis();
    let delta = i64::try_from(dur.as_millis()).unwrap_or(i64::MAX);
    Ok(now.saturating_sub(delta))
}

fn parse_kind(s: &str) -> Result<CardKind> {
    Ok(match s {
        "hook" => CardKind::HookFailure,
        "hook-slow" => CardKind::HookSlow,
        "hook-info" => CardKind::HookGuidance,
        "agent" => CardKind::AgentReturn,
        "agent-stranded" => CardKind::AgentStranded,
        "tool-error" => CardKind::ToolError,
        "command" => CardKind::CommandFailure,
        "milestone" => CardKind::SessionMilestone,
        other => anyhow::bail!(
            "unknown --kind {other:?}; valid values: hook, hook-slow, hook-info, agent, agent-stranded, tool-error, command, milestone"
        ),
    })
}

fn parse_severity(s: &str) -> Result<Severity> {
    Ok(match s.to_ascii_lowercase().as_str() {
        "info" => Severity::Info,
        "notice" => Severity::Notice,
        "warn" | "warning" => Severity::Warn,
        "error" | "err" => Severity::Error,
        other => {
            anyhow::bail!("unknown --severity {other:?}; valid values: info, notice, warn, error")
        }
    })
}

/// Human renderer — mirrors design v2 §9 mock. Fixed columns, monospace
/// friendly. Two lines per card: title row + optional help row prefixed
/// with `↳`.
fn print_human(cards: &[Card]) {
    for c in cards {
        let local: DateTime<Local> = c.ts.with_timezone(&Local);
        let ts = local.format("%m-%d %H:%M");
        let project = project_label(&c.cwd);
        let branch = c
            .git_branch
            .as_deref()
            .map(|b| format!(" · {b}"))
            .unwrap_or_default();

        println!(
            "{ts}  {sev:6} {kind:<13} {title}",
            sev = c.severity.label(),
            kind = c.kind.label(),
            title = c.title,
        );
        if let Some(sub) = &c.subtitle {
            println!("              {sub}");
        }
        if let Some(help) = &c.help {
            if let Some(rendered) = render_help(help) {
                println!("              ↳ {rendered}");
            }
        }
        let plugin = c
            .plugin
            .as_deref()
            .map(|p| format!(" · plugin:{p}"))
            .unwrap_or_default();
        println!("              ({project}{branch}{plugin})");
        println!();
    }
}

/// Best-effort short label for a project cwd: the basename. Avoids
/// printing absolute paths in the human renderer (they wrap in
/// terminals); the JSON output always carries the full path.
fn project_label(cwd: &std::path::Path) -> String {
    cwd.file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| cwd.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_since_accepts_common_units() {
        for input in ["1s", "30m", "2h", "7d"] {
            let out = parse_since(input).unwrap();
            assert!(out > 0, "{input} should produce a positive timestamp");
        }
    }

    #[test]
    fn parse_since_rejects_bad_input() {
        for input in ["", "abc", "10x", "h", "1.5h"] {
            assert!(parse_since(input).is_err(), "{input} should be rejected");
        }
    }

    #[test]
    fn parse_kind_round_trips_label() {
        // The CLI label and the CardKind::label() value MUST match —
        // otherwise filtering by what the human renderer prints fails.
        for k in [
            CardKind::HookFailure,
            CardKind::HookSlow,
            CardKind::AgentReturn,
            CardKind::ToolError,
            CardKind::CommandFailure,
            CardKind::SessionMilestone,
        ] {
            let parsed = parse_kind(k.label()).unwrap();
            assert_eq!(parsed, k, "label round trip failed for {:?}", k);
        }
    }

    #[test]
    fn parse_severity_accepts_aliases() {
        assert_eq!(parse_severity("warn").unwrap(), Severity::Warn);
        assert_eq!(parse_severity("WARNING").unwrap(), Severity::Warn);
        assert_eq!(parse_severity("err").unwrap(), Severity::Error);
        assert_eq!(parse_severity("Info").unwrap(), Severity::Info);
        assert!(parse_severity("critical").is_err());
    }
}
