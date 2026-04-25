//! Tauri commands for the activity *cards* surface.
//!
//! Pure pass-through to `claudepot_core::activity::ActivityIndex`.
//! No business logic — each command parses its DTO, calls one core
//! function, and serializes the result. The cards index is exposed
//! as Tauri-managed state from `lib.rs::run`.
//!
//! Heavy work runs on the Tokio blocking pool (SQLite reads can
//! stall the IPC worker on a hot DB or large index). Commands that
//! only flip a meta cell stay sync.

use std::io::{Read, Seek, SeekFrom};
use std::sync::Arc;

use claudepot_core::activity::ActivityIndex;
use tauri::State;

use crate::dto_activity_cards::{
    ActivityCardDto, CardNavigateDto, CardsCountDto, CardsRecentQueryDto, CardsReindexFailureDto,
    CardsReindexResultDto,
};

/// Tauri-managed handle to the activity index. Wrapped so the
/// command surface only depends on this type, not directly on
/// `ActivityIndex`. Lets test harnesses inject a tempdir-backed
/// index without touching production paths.
pub struct ActivityCardsState {
    pub index: Arc<ActivityIndex>,
}

/// Recent cards. The default render path of the GUI's card stream.
/// Pushed onto the blocking pool so a slow SQLite query doesn't
/// pin the IPC worker.
#[tauri::command]
pub async fn cards_recent(
    state: State<'_, ActivityCardsState>,
    query: CardsRecentQueryDto,
) -> Result<Vec<ActivityCardDto>, String> {
    let q = query.into_core()?;
    let idx = Arc::clone(&state.index);
    let cards = tauri::async_runtime::spawn_blocking(move || idx.recent(&q))
        .await
        .map_err(|e| format!("recent join: {e}"))?
        .map_err(|e| e.to_string())?;
    Ok(cards.iter().map(ActivityCardDto::from).collect())
}

/// Total + new-since-cursor counts. Drives the "N new since you
/// were away" badge on the GUI strip.
#[tauri::command]
pub async fn cards_count_new_since(
    state: State<'_, ActivityCardsState>,
    query: CardsRecentQueryDto,
) -> Result<CardsCountDto, String> {
    let q = query.into_core()?;
    let idx = Arc::clone(&state.index);
    tauri::async_runtime::spawn_blocking(move || -> Result<CardsCountDto, String> {
        let last_seen = idx.last_seen().map_err(|e| e.to_string())?;
        let total = idx.count_new_since(None, &q).map_err(|e| e.to_string())?;
        let new = idx
            .count_new_since(last_seen, &q)
            .map_err(|e| e.to_string())?;
        Ok(CardsCountDto {
            total,
            new,
            last_seen_id: last_seen,
        })
    })
    .await
    .map_err(|e| format!("count join: {e}"))?
}

/// Set the cursor to `card_id`. Idempotent. Called when the user
/// scrolls past unread cards or explicitly marks-as-seen.
#[tauri::command]
pub fn cards_set_last_seen(
    state: State<'_, ActivityCardsState>,
    card_id: i64,
) -> Result<(), String> {
    state.index.set_last_seen(card_id).map_err(|e| e.to_string())
}

/// Resolve a card id to a navigation payload — session path +
/// byte offset + event uuid. The GUI consumes this to switch to
/// the Sessions section and scroll to the matching line.
#[tauri::command]
pub async fn cards_navigate(
    state: State<'_, ActivityCardsState>,
    card_id: i64,
) -> Result<Option<CardNavigateDto>, String> {
    let idx = Arc::clone(&state.index);
    tauri::async_runtime::spawn_blocking(move || -> Result<Option<CardNavigateDto>, String> {
        // Recent query with limit 1 won't work — we need an
        // arbitrary id lookup. Use a pinpoint query through the
        // recent path with limit 10000 and find by id; cheap because
        // every recent card carries `id` and the typical caller
        // already loaded the same row in the GUI list.
        // For a more direct path, expose a `get_by_id` on the
        // index later; v1 stays minimal.
        let cards = idx
            .recent(&claudepot_core::activity::RecentQuery {
                limit: Some(10_000),
                ..Default::default()
            })
            .map_err(|e| e.to_string())?;
        Ok(cards
            .into_iter()
            .find(|c| c.id == Some(card_id))
            .map(|c| CardNavigateDto {
                session_path: c.session_path.to_string_lossy().into_owned(),
                byte_offset: c.byte_offset,
                event_uuid: c.event_uuid,
            }))
    })
    .await
    .map_err(|e| format!("navigate join: {e}"))?
}

/// Fetch the body of a card lazily — seek to `byte_offset` in the
/// source JSONL, read one line, redact, return. Cap at 4 KiB to
/// keep IPC frames small (the design's stated trade-off — body is
/// pulled lazily, not stored).
#[tauri::command]
pub async fn cards_body(
    state: State<'_, ActivityCardsState>,
    card_id: i64,
) -> Result<Option<String>, String> {
    let nav = cards_navigate(state, card_id).await?;
    let Some(nav) = nav else {
        return Ok(None);
    };
    let path = nav.session_path;
    let off = nav.byte_offset;
    let body = tauri::async_runtime::spawn_blocking(move || -> std::io::Result<Option<String>> {
        let mut f = std::fs::File::open(&path)?;
        f.seek(SeekFrom::Start(off))?;
        let mut buf = vec![0u8; 4096];
        let n = f.read(&mut buf)?;
        if n == 0 {
            return Ok(None);
        }
        // Truncate to first newline so we return one line, not the
        // tail of the file. Keep buf trimmed and redacted.
        let line = match buf[..n].iter().position(|b| *b == b'\n') {
            Some(idx) => &buf[..idx],
            None => &buf[..n],
        };
        let s = String::from_utf8_lossy(line).into_owned();
        Ok(Some(claudepot_core::session_live::redact::redact_secrets(
            &s,
        )))
    })
    .await
    .map_err(|e| format!("body join: {e}"))?
    .map_err(|e| format!("body io: {e}"))?;
    Ok(body)
}

/// Trigger a backfill. Heavy I/O — runs on the blocking pool.
/// Returns the same shape the CLI prints, with a per-failure path
/// list (capped at 50 entries to keep the IPC frame small).
#[tauri::command]
pub async fn cards_reindex(
    state: State<'_, ActivityCardsState>,
) -> Result<CardsReindexResultDto, String> {
    let idx = Arc::clone(&state.index);
    let stats = tauri::async_runtime::spawn_blocking(move || {
        let config_dir = claudepot_core::paths::claude_config_dir();
        claudepot_core::activity::backfill::run(&config_dir, &idx)
    })
    .await
    .map_err(|e| format!("reindex join: {e}"))?
    .map_err(|e| e.to_string())?;
    Ok(CardsReindexResultDto {
        files_scanned: stats.files_scanned,
        cards_inserted: stats.cards_inserted,
        cards_skipped_duplicates: stats.cards_skipped_duplicates,
        cards_pruned: stats.cards_pruned,
        failed: stats
            .failed
            .into_iter()
            .take(50)
            .map(|(p, e)| CardsReindexFailureDto {
                path: p.to_string_lossy().into_owned(),
                error: e,
            })
            .collect(),
        elapsed_ms: stats.elapsed.as_millis() as u64,
    })
}
