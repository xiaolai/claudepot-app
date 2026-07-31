//! Board lifecycle verbs: `open`, `list`, `get`, `rm`.

use super::*;
use crate::output::print_json;
use crate::AppContext;
use claudepot_core::board::{BoardSpec, SeriesDef};
use serde::{Deserialize, Serialize};

/// The `open` payload. A board is defined by three pieces, and passing
/// them as one JSON document keeps a definition diffable and
/// re-runnable — which is what a scheduled agent needs.
#[derive(Debug, Deserialize)]
pub struct BoardDraft {
    pub name: String,
    pub series: Vec<SeriesDef>,
    #[serde(default = "BoardSpec::empty")]
    pub spec: BoardSpec,
}

#[derive(Serialize)]
struct BoardSummary<'a> {
    board_id: &'a str,
    name: &'a str,
    spec_revision: i64,
    created_at: String,
    updated_at: String,
    series: Vec<&'a str>,
}

/// `board open --from <file|-> [--name <override>]`
pub fn open_cmd(ctx: &AppContext, from: &str, name_override: Option<String>) -> Result<()> {
    let raw = read_input(from)?;
    let mut draft: BoardDraft =
        serde_json::from_str(&raw).context("parsing board definition JSON")?;
    if let Some(n) = name_override {
        draft.name = n;
    }

    let store = store()?;
    let board = store
        .create_board(&draft.name, &draft.spec, &draft.series)
        .context("creating board")?;

    if ctx.json {
        print_json(&serde_json::json!({
            "board_id": board.board_id,
            "name": board.name,
            "spec_revision": board.spec_revision,
        }))?;
    } else {
        // The id is the identity; the name is a mutable label. Print
        // the id so a script can capture it.
        println!("{}", board.board_id);
        if !ctx.quiet {
            eprintln!(
                "created board `{}` with {} series",
                board.name,
                draft.series.len()
            );
        }
    }
    Ok(())
}

/// `board list`
pub fn list_cmd(ctx: &AppContext) -> Result<()> {
    let store = store()?;
    let boards = store.list_boards()?;

    if ctx.json {
        let mut out = Vec::new();
        for b in &boards {
            let series = store.series_defs(&b.board_id)?;
            out.push(serde_json::json!({
                "board_id": b.board_id,
                "name": b.name,
                "spec_revision": b.spec_revision,
                "updated_at": b.updated_at.to_rfc3339(),
                "series": series.iter().map(|s| &s.name).collect::<Vec<_>>(),
            }));
        }
        return print_json(&out);
    }

    if boards.is_empty() {
        // Render-if-nonzero: an empty list says so once, not as a table
        // with zero rows and a header.
        if !ctx.quiet {
            eprintln!("no boards");
        }
        return Ok(());
    }

    for b in &boards {
        let series = store.series_defs(&b.board_id)?;
        let names: Vec<&str> = series.iter().map(|s| s.name.as_str()).collect();
        println!(
            "{}  {}  updated {}  [{}]",
            b.board_id,
            b.name,
            b.updated_at.to_rfc3339(),
            names.join(", ")
        );
    }
    Ok(())
}

/// `board get <board-id>` — spec and its revision, the precondition for
/// any later patch.
pub fn get_cmd(ctx: &AppContext, board_id: &str) -> Result<()> {
    let store = store()?;
    let board = store.get_board(board_id)?;
    let series = store.series_defs(board_id)?;

    if ctx.json {
        let summary = BoardSummary {
            board_id: &board.board_id,
            name: &board.name,
            spec_revision: board.spec_revision,
            created_at: board.created_at.to_rfc3339(),
            updated_at: board.updated_at.to_rfc3339(),
            series: series.iter().map(|s| s.name.as_str()).collect(),
        };
        return print_json(&serde_json::json!({
            "board": summary,
            "spec": board.spec,
            "series_defs": series,
        }));
    }

    println!("board    {}", board.board_id);
    println!("name     {}", board.name);
    println!("revision {}", board.spec_revision);
    println!("created  {}", board.created_at.to_rfc3339());
    println!("updated  {}", board.updated_at.to_rfc3339());
    if let Some(src) = store.source_board_id(board_id)? {
        println!("imported from {src}");
    }
    for def in &series {
        let cols: Vec<String> = def
            .columns
            .iter()
            .map(|c| format!("{}:{}", c.name, c.ty.as_str()))
            .collect();
        let n = store.row_count(board_id, &def.name)?;
        println!("series   {} ({}) — {} rows", def.name, cols.join(", "), n);
    }
    for w in &board.spec.widgets {
        println!("widget   {} {} -> {}", w.id, w.kind.as_str(), w.series);
    }
    Ok(())
}

/// `board rm <board-id>` — explicit deletion, the only kind there is.
pub fn rm_cmd(ctx: &AppContext, board_id: &str) -> Result<()> {
    let store = store()?;
    let board = store.get_board(board_id)?;

    // Boards are user data with no automatic pruning (plan §12), so a
    // delete is unrecoverable — confirm unless told not to.
    if !ctx.yes {
        let series = store.series_defs(board_id)?;
        let mut rows = 0usize;
        for def in &series {
            rows += store.row_count(board_id, &def.name)?;
        }
        anyhow::bail!(
            "refusing to delete board `{}` ({} rows across {} series) without --yes. \
             Export it first: claudepot experimental board export {} --format json --out board.json",
            board.name,
            rows,
            series.len(),
            board_id
        );
    }

    store.delete_board(board_id)?;
    if !ctx.quiet {
        eprintln!("deleted board `{}`", board.name);
    }
    Ok(())
}
