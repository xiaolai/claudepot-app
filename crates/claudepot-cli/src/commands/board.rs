//! `claudepot experimental board` — shared helpers and re-exports.
//!
//! # Why `experimental`
//!
//! Boards are on trial (plan §10.1): a real scheduled agent writes to
//! one for a week against fixed pass/fail criteria, and if it fails,
//! the core module, the DB file, and these verbs are deleted. Shipping
//! them as `claudepot board …` would create a stable CLI contract that
//! outlives the experiment and makes the deletion expensive — which is
//! precisely the sunk cost the trial exists to avoid.
//!
//! They graduate to `claudepot board …` when the trial passes.
//!
//! Thin wrapper only: every verb here parses arguments, calls
//! `claudepot_core::board`, and formats. No business logic, per
//! `rules/commands.md`.

use anyhow::{Context, Result};
use claudepot_core::board::{boards_db_path, BoardStore, Row, SeriesDef, WriterKind};

pub mod data;
pub mod lifecycle;
pub mod transfer;

pub use data::{clear_cmd, push_cmd, show_cmd};
pub use lifecycle::{get_cmd, list_cmd, open_cmd, rm_cmd};
pub use transfer::{export_cmd, import_cmd};

/// Open the boards store at its standard path.
///
/// Every writer opens this file directly — there is no daemon to reach
/// and nothing to authenticate against. See
/// `claudepot_core::board`'s module docs for why, and for the cost.
pub(crate) fn store() -> Result<BoardStore> {
    let path = boards_db_path();
    BoardStore::open(&path).with_context(|| format!("opening {}", path.display()))
}

/// Resolve a JSON argument from one of three forms:
///
/// - `-` — read stdin, for pipelines.
/// - a string starting with `[` or `{` — literal JSON, so a small push
///   needs no temp file. This is the common case for a shell script
///   appending three rows, and requiring a file for it made the verb
///   annoying enough to route around.
/// - anything else — a filesystem path.
///
/// **An existing file always wins.** The path is tested first, so a
/// file named `[weird].json` is reachable by its bare name — a
/// prefix-first rule made any such file unopenable, and a `./` prefix
/// is not something a user can be expected to guess. Only when no such
/// file exists does the `[`/`{` prefix mean "this is literal JSON".
///
/// The literal-JSON test stays a prefix check rather than a parse
/// attempt: a parse-first rule would report a malformed row array as
/// "file not found", which is the error that hid this whole shape from
/// the first smoke test.
pub(crate) fn read_input(arg: &str) -> Result<String> {
    if arg == "-" {
        use std::io::Read;
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .context("reading stdin")?;
        return Ok(buf);
    }

    if std::path::Path::new(arg).is_file() {
        return std::fs::read_to_string(arg).with_context(|| format!("reading {arg}"));
    }

    let trimmed = arg.trim_start();
    if trimmed.starts_with('[') || trimmed.starts_with('{') {
        return Ok(arg.to_string());
    }

    anyhow::bail!("no file at `{arg}` — pass a path, inline JSON, or `-` for stdin")
}

/// Parse a `--writer-kind` value.
///
/// The value is **self-declared**. Claudepot records what a writer says
/// it is and cannot verify it (plan §8.5), which is why every rendering
/// path below says "Reported by".
pub(crate) fn parse_writer_kind(raw: &str) -> Result<WriterKind> {
    WriterKind::parse(raw).with_context(|| {
        format!("unknown writer kind `{raw}` (agent_run, cc_session, cli, import, system)")
    })
}

/// Render rows as a fixed-width terminal table.
///
/// Provenance columns are prefixed `reported.` rather than merged in
/// with the data, so a reader cannot mistake a writer's claim about
/// itself for something the agent computed.
pub(crate) fn render_table(def: &SeriesDef, rows: &[Row]) -> String {
    let mut headers: Vec<String> = def.columns.iter().map(|c| c.name.clone()).collect();
    headers.push("reported.writer".to_string());
    headers.push("reported.at".to_string());

    let mut table: Vec<Vec<String>> = vec![headers.clone()];
    for row in rows {
        let mut cells: Vec<String> = row.values.iter().map(|v| v.to_display()).collect();
        cells.push(row.provenance.writer.label.clone());
        cells.push(row.provenance.pushed_at.to_rfc3339());
        table.push(cells);
    }

    let widths: Vec<usize> = (0..headers.len())
        .map(|i| {
            table
                .iter()
                .map(|r| r.get(i).map_or(0, |c| c.chars().count()))
                .max()
                .unwrap_or(0)
        })
        .collect();

    let mut out = String::new();
    for (n, row) in table.iter().enumerate() {
        let line: Vec<String> = row
            .iter()
            .enumerate()
            .map(|(i, cell)| {
                let pad = widths[i].saturating_sub(cell.chars().count());
                format!("{cell}{}", " ".repeat(pad))
            })
            .collect();
        out.push_str(line.join("  ").trim_end());
        out.push('\n');
        if n == 0 {
            let rule: Vec<String> = widths.iter().map(|w| "-".repeat(*w)).collect();
            out.push_str(rule.join("  ").trim_end());
            out.push('\n');
        }
    }
    out
}
