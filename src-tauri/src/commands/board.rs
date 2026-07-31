//! Tauri commands for the Boards sub-surface (Activities → Boards).
//!
//! Thin wrappers over `claudepot_core::board`. No business logic — the
//! degenerate-data guards, caps, and provenance rules all live in core
//! so the CLI and this surface cannot disagree about them.
//!
//! Every handler is `async fn` for the reason in `commands/mod.rs`:
//! blocking SQLite I/O on Tauri's main thread freezes the webview.
//!
//! # Provenance crosses this bridge as a claim
//!
//! There is no authenticated write channel (plan §11), so `writer_id`
//! is self-reported. Every DTO field carrying it is named `reported_*`
//! — the name is the guard, because a renderer that wants to present
//! it as fact has to rename it first. There is deliberately no
//! `verified` boolean: a field that is always `false` invites a
//! surface to branch on it and eventually to set it. See plan §8.5.

use claudepot_core::board::{boards_db_path, widget, BoardStore, ResolvedWidget};
use serde::Serialize;
use std::sync::{Arc, Mutex, OnceLock};

/// One long-lived store for the whole process.
///
/// **`PRAGMA data_version` is per-connection.** It reports whether
/// *another* connection has committed, so comparing values taken from
/// two different connections is meaningless — a fresh `BoardStore::open`
/// per poll always looked unchanged, which silently disabled live
/// refresh. `board::monitor`'s own docs say the connection must be
/// long-lived; this is where that gets honored.
///
/// Sharing it also stops the UI from opening a SQLite handle every four
/// seconds for the poll.
static STORE: OnceLock<Mutex<Option<Arc<BoardStore>>>> = OnceLock::new();

fn open() -> Result<Arc<BoardStore>, String> {
    let cell = STORE.get_or_init(|| Mutex::new(None));
    let mut guard = cell.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(existing) = guard.as_ref() {
        return Ok(existing.clone());
    }
    // A failed open is NOT cached: the data dir may not exist yet on a
    // first run, and caching the error would make the surface
    // permanently dead until restart.
    let store = Arc::new(
        BoardStore::open(&boards_db_path())
            .map_err(|e| format!("boards store open failed: {e}"))?,
    );
    *guard = Some(store.clone());
    Ok(store)
}

/// One row in the boards table.
#[derive(Serialize)]
pub struct BoardSummaryDto {
    pub board_id: String,
    pub name: String,
    pub spec_revision: i64,
    pub created_at: String,
    pub updated_at: String,
    pub series: Vec<String>,
    pub total_rows: usize,
    /// Human label for the most recent writer, already phrased as a
    /// claim. `None` when the board has no rows yet.
    pub reported_writer: Option<String>,
}

/// A board plus its widgets already resolved into render plans.
#[derive(Serialize)]
pub struct BoardDetailDto {
    pub board_id: String,
    pub name: String,
    pub spec_revision: i64,
    pub created_at: String,
    pub updated_at: String,
    pub source_board_id: Option<String>,
    pub series: Vec<SeriesSummaryDto>,
    pub widgets: Vec<ResolvedWidget>,
    /// Rendered verbatim by the UI so the caveat cannot drift from the
    /// backend that knows it is true.
    pub provenance_note: String,
}

#[derive(Serialize)]
pub struct SeriesSummaryDto {
    pub name: String,
    pub columns: Vec<ColumnDto>,
    pub row_count: usize,
    pub reported_writer: Option<String>,
    pub last_pushed_at: Option<String>,
}

#[derive(Serialize)]
pub struct ColumnDto {
    pub name: String,
    pub ty: String,
}

const PROVENANCE_NOTE: &str =
    "Writers are self-declared. Claudepot does not authenticate them, so \
     every name below is what a writer claimed, not verified identity.";

/// Rows read per series when resolving widgets.
///
/// Core downsamples and truncates beyond this anyway; the cap keeps a
/// million-row series from crossing the IPC bridge to be thrown away.
const READ_CAP: usize = 20_000;

#[tauri::command]
pub async fn board_list() -> Result<Vec<BoardSummaryDto>, String> {
    let store = open()?;
    // One batched call, not `1 + boards × series` round trips.
    let summaries = store.list_summaries().map_err(|e| e.to_string())?;
    Ok(summaries
        .into_iter()
        .map(|s| BoardSummaryDto {
            board_id: s.board.board_id,
            name: s.board.name,
            spec_revision: s.board.spec_revision,
            created_at: s.board.created_at.to_rfc3339(),
            updated_at: s.board.updated_at.to_rfc3339(),
            series: s.series,
            total_rows: s.total_rows,
            reported_writer: s.reported_writer,
        })
        .collect())
}

#[tauri::command]
pub async fn board_detail(board_id: String) -> Result<BoardDetailDto, String> {
    let store = open()?;
    // ONE transactional snapshot. Assembling this from separate reads
    // let a concurrent writer land between them, so the row count could
    // disagree with the rows shipped beside it.
    let snap = store
        .detail_snapshot(&board_id, READ_CAP)
        .map_err(|e| e.to_string())?;

    let mut series = Vec::with_capacity(snap.series.len());
    let mut widgets = Vec::new();

    for s in &snap.series {
        series.push(SeriesSummaryDto {
            name: s.def.name.clone(),
            columns: s
                .def
                .columns
                .iter()
                .map(|c| ColumnDto {
                    name: c.name.clone(),
                    ty: c.ty.as_str().to_string(),
                })
                .collect(),
            row_count: s.row_count,
            reported_writer: s.rows.last().map(|r| r.provenance.writer.label.clone()),
            last_pushed_at: s.rows.last().map(|r| r.provenance.pushed_at.to_rfc3339()),
        });

        for slot in snap
            .board
            .spec
            .widgets
            .iter()
            .filter(|w| w.series == s.def.name)
        {
            widgets.push(widget::resolve(slot, &s.def, &s.rows, false));
        }

        // A board with series but no widgets is legal — the CLI trial
        // works that way. Synthesize one table per series from the rows
        // already in hand.
        if snap.board.spec.widgets.is_empty() {
            widgets.push(widget::resolve(
                &claudepot_core::board::WidgetSlot {
                    id: format!("auto-{}", s.def.name),
                    kind: claudepot_core::board::WidgetKind::Table,
                    series: s.def.name.clone(),
                    title: s.def.name.clone(),
                    x_column: None,
                    y_column: None,
                },
                &s.def,
                &s.rows,
                false,
            ));
        }
    }

    Ok(BoardDetailDto {
        board_id: snap.board.board_id,
        name: snap.board.name,
        spec_revision: snap.board.spec_revision,
        created_at: snap.board.created_at.to_rfc3339(),
        updated_at: snap.board.updated_at.to_rfc3339(),
        source_board_id: snap.source_board_id,
        series,
        widgets,
        provenance_note: PROVENANCE_NOTE.to_string(),
    })
}

/// `PRAGMA data_version` for change polling.
///
/// The UI polls this and re-fetches a snapshot when it moves. There is
/// no delta — see `board::monitor` for why a watcher is the wrong tool
/// and why `data_version` gives no diff.
#[tauri::command]
pub async fn board_data_version() -> Result<i64, String> {
    let store = open()?;
    store.data_version().map_err(|e| e.to_string())
}

/// Delete a board and all its data. Explicit only — nothing prunes.
#[tauri::command]
pub async fn board_delete(board_id: String) -> Result<(), String> {
    let store = open()?;
    store.delete_board(&board_id).map_err(|e| e.to_string())
}

/// Export a board to a path chosen by the caller.
/// Export a board to a path the **user** chose.
///
/// The path must be absolute. The renderer previously passed a bare
/// filename, which resolved against the app's working directory —
/// wherever the OS happened to launch it from. A save dialog picks the
/// destination; this command refuses anything else rather than guessing.
#[tauri::command]
pub async fn board_export(board_id: String, path: String) -> Result<String, String> {
    let target = std::path::Path::new(&path);
    if !target.is_absolute() {
        return Err("export path must be absolute — pick a destination first".to_string());
    }
    let store = open()?;
    // `create_new` makes "must not exist" atomic. An `exists()` check
    // followed by `File::create` is a TOCTOU window that truncates a
    // file created in between — and this writes user data.
    let file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .map_err(|e| match e.kind() {
            std::io::ErrorKind::AlreadyExists => format!("{path} already exists"),
            _ => format!("creating {path}: {e}"),
        })?;
    let mut w = std::io::BufWriter::new(file);
    claudepot_core::board::export_json(&store, &board_id, &mut w).map_err(|e| e.to_string())?;
    Ok(path)
}
