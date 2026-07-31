//! Data verbs: `push`, `show`, `clear`.

use super::*;
use crate::output::print_json;
use crate::AppContext;
use claudepot_core::board::{PushMode, PushRequest, WriterId};

/// Flags for `board push`, grouped so the handler avoids a
/// `too_many_arguments` allow (see `rules/commands.md`).
#[derive(Debug, clap::Args)]
pub struct PushArgs {
    /// Board id (from `board open` or `board list`)
    pub board_id: String,

    /// Series to append to
    #[arg(long)]
    pub series: String,

    /// Rows as inline JSON, a file path, or `-` for stdin. Each row is
    /// an array of cells matching the series' column order.
    #[arg(long)]
    pub rows: String,

    /// `append` (default) or `replace`
    #[arg(long, default_value = "append")]
    pub mode: String,

    /// Self-declared writer label, shown as "Reported by …"
    #[arg(long, default_value = "cli")]
    pub writer: String,

    /// Self-declared writer kind: agent_run, cc_session, cli, import, system
    #[arg(long = "writer-kind", default_value = "cli")]
    pub writer_kind: String,

    /// Dedup key. Re-pushing the same key is a no-op, so a retried cron
    /// job cannot double-append.
    #[arg(long = "idem-key")]
    pub idem_key: Option<String>,

    /// Explicit starting sequence for this writer. Omit to continue
    /// after its last row.
    #[arg(long = "writer-seq")]
    pub writer_seq: Option<i64>,
}

/// `board push`
pub fn push_cmd(ctx: &AppContext, args: &PushArgs) -> Result<()> {
    let raw = read_input(&args.rows)?;
    let rows: Vec<serde_json::Value> =
        serde_json::from_str(&raw).context("parsing rows as a JSON array of arrays")?;
    let mode = PushMode::parse(&args.mode)
        .with_context(|| format!("unknown mode `{}` (append or replace)", args.mode))?;
    let kind = parse_writer_kind(&args.writer_kind)?;

    let store = store()?;
    let outcome = store.push(&PushRequest {
        board_id: args.board_id.clone(),
        series: args.series.clone(),
        rows,
        mode,
        writer: WriterId::new(kind, args.writer.clone()),
        idem_key: args.idem_key.clone(),
        writer_seq: args.writer_seq,
    })?;

    if ctx.json {
        return print_json(&serde_json::json!({
            "rows_added": outcome.rows_added,
            "deduplicated": outcome.deduplicated,
            "sequence_gap": outcome.sequence_gap.map(|(a, b)| [a, b]),
        }));
    }

    if !ctx.quiet {
        if outcome.deduplicated {
            eprintln!("no-op: idem-key already applied");
        } else {
            eprintln!("appended {} rows to `{}`", outcome.rows_added, args.series);
        }
        // A gap is informational, not fatal — the rows that arrived are
        // real. But it must be visible, or a reader renders an
        // incomplete series as complete.
        if let Some((first, last)) = outcome.sequence_gap {
            eprintln!(
                "warning: writer `{}` skipped sequence {}..={} on `{}`",
                args.writer, first, last, args.series
            );
        }
    }
    Ok(())
}

/// `board show <board-id> [--series <name>] [--limit N]`
///
/// The terminal renderer. During the plan §10.1 trial this is the whole
/// user interface, which is the point: it proves the data layer without
/// committing to a GUI.
pub fn show_cmd(
    ctx: &AppContext,
    board_id: &str,
    series: Option<String>,
    limit: usize,
) -> Result<()> {
    let store = store()?;
    let board = store.get_board(board_id)?;
    let defs = store.series_defs(board_id)?;

    let selected: Vec<&SeriesDef> = match &series {
        Some(name) => {
            let found = defs
                .iter()
                .find(|d| &d.name == name)
                .with_context(|| format!("board has no series `{name}`"))?;
            vec![found]
        }
        None => defs.iter().collect(),
    };

    if ctx.json {
        let mut out = Vec::new();
        for def in &selected {
            let rows = store.read_rows(board_id, &def.name, limit)?;
            out.push(serde_json::json!({
                "series": def.name,
                "columns": def.columns,
                "total_rows": store.row_count(board_id, &def.name)?,
                // `show` is a DISPLAY surface, `--json` included — it is
                // what a human or a script pipes to a terminal. Use the
                // redacting path; `board export` is the fidelity path.
                // Emitting `to_json()` here bypassed the split entirely
                // and put raw cells back on stdout.
                "rows": rows.iter().map(|r| serde_json::json!({
                    "values": r.values.iter().map(|v| v.to_display()).collect::<Vec<_>>(),
                    "writer_seq": r.writer_seq,
                    // Never "writer" alone — the key names the claim.
                    "reported_writer": r.provenance.writer.label,
                    "reported_writer_kind": r.provenance.writer.kind.as_str(),
                    "verified": r.provenance.verified,
                    "pushed_at": r.provenance.pushed_at.to_rfc3339(),
                })).collect::<Vec<_>>(),
            }));
        }
        return print_json(&serde_json::json!({
            "board_id": board.board_id,
            "name": board.name,
            "provenance_note": "reported_writer is self-declared and unverified",
            "series": out,
        }));
    }

    println!("{}  ({})", board.name, board.board_id);
    for def in &selected {
        let total = store.row_count(board_id, &def.name)?;
        let rows = store.read_rows(board_id, &def.name, limit)?;
        println!();
        if total == 0 {
            // Empty is a state, not a blank table.
            println!("{} — no rows yet", def.name);
            continue;
        }
        if total > rows.len() {
            // Never a silent truncation.
            println!("{} — showing {} of {} rows", def.name, rows.len(), total);
        } else {
            println!("{} — {} rows", def.name, total);
        }
        print!("{}", render_table(def, &rows));
    }
    Ok(())
}

/// `board clear <board-id> --series <name>`
pub fn clear_cmd(ctx: &AppContext, board_id: &str, series: &str) -> Result<()> {
    let store = store()?;
    if !ctx.yes {
        let n = store.row_count(board_id, series)?;
        anyhow::bail!(
            "refusing to clear {n} rows from `{series}` without --yes. \
             Export first: claudepot experimental board export {board_id} --format json --out board.json"
        );
    }
    let removed = store.clear_series(board_id, series)?;
    if !ctx.quiet {
        eprintln!("cleared {removed} rows from `{series}`");
    }
    Ok(())
}
