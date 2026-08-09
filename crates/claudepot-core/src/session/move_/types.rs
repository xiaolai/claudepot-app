//! Types + shared constants for the `move_` boundary.
//!
//! Kept in a separate submodule per loc-guardian rule 1 (types >30 LOC
//! extract to their own module).

use std::path::PathBuf;
use std::time::Duration;
use thiserror::Error;
use uuid::Uuid;

/// Invalid-slug sentinel used by both the library guard and the CLI/Tauri
/// wrappers. Slugs are produced by `sanitize_path`, which emits only
/// `[A-Za-z0-9-]+`. Any slug outside that alphabet is untrusted input
/// (most likely a path-traversal attempt); callers must be rejected at
/// the library boundary regardless of whether the resulting path happens
/// to be a real directory. See `super::helpers::validate_slug`.
pub const INVALID_SLUG_MSG: &str = "slug must be a single non-empty directory name \
containing only alphanumerics, hyphens, or underscores";

/// Threshold below which we treat the source session file as "live" —
/// i.e., CC may currently be writing to it. Matches the project_lock
/// heartbeat semantics elsewhere in the crate (2s is aggressive but safe
/// given CC's per-turn flush cadence).
pub(crate) const LIVE_SESSION_MTIME_THRESHOLD: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Default)]
pub struct MoveSessionOpts {
    /// Proceed even if the source JSONL's mtime is within the live-session
    /// threshold. Default false — refuse to move a session CC may be writing.
    pub force_live_session: bool,
    /// Proceed even if a Syncthing `.sync-conflict-*.jsonl` sibling exists.
    /// Default false — refuse to silently discard conflict state.
    pub force_sync_conflict: bool,
    /// After a successful move, remove the source project dir if it now
    /// contains no JSONL files and no session subdirs.
    pub cleanup_source_if_empty: bool,
    /// Create `to_cwd` on disk when it does not exist yet. Default false —
    /// orphan adoption, the original caller, targets a directory it has
    /// already verified is there.
    ///
    /// This is what makes "move this session into a folder I haven't
    /// created yet" a supported operation rather than a silent way to
    /// manufacture a fresh orphan: without it the transcript's `cwd` is
    /// rewritten to a path `--resume` cannot `cd` into.
    pub create_target_dir: bool,
    /// Path to the `~/.claude.json` config file where CC stores per-project
    /// metadata (including `lastSessionId` and
    /// `activeWorktreeSession.sessionId` session pointers). The caller
    /// must supply this — there is no "default" location because
    /// production is `$HOME/.claude.json` (a sibling of `$HOME/.claude/`)
    /// while tests locate it elsewhere. `None` skips Phase 4 entirely.
    pub claude_json_path: Option<PathBuf>,
}

#[derive(Debug, Default, Clone)]
pub struct MoveSessionReport {
    pub session_id: Option<Uuid>,
    pub from_slug: String,
    pub to_slug: String,
    /// Count of lines whose `cwd` field was rewritten in the primary
    /// session JSONL. Matches total line count for sessions whose every
    /// line stayed in one cwd (the normal case).
    pub jsonl_lines_rewritten: usize,
    /// Count of files moved from `<slug_from>/<S>/subagents/**`.
    pub subagent_files_moved: usize,
    /// Count of files moved from `<slug_from>/<S>/remote-agents/**`.
    pub remote_agent_files_moved: usize,
    /// Count of history.jsonl lines whose `project` field was rewritten
    /// (lines where `sessionId == S` AND `project == canonical(from_cwd)`).
    pub history_entries_moved: usize,
    /// Count of history.jsonl lines that likely belong to this session
    /// but could not be attributed — typically pre-sessionId CC versions.
    /// Left as-is.
    pub history_entries_unmapped: usize,
    /// 0, 1, or 2: how many of the two possible `.claude.json` session
    /// pointers were pointing at this session and had to be cleared.
    pub claude_json_pointers_cleared: u8,
    /// True iff `opts.cleanup_source_if_empty` and the source dir was empty.
    pub source_dir_removed: bool,
}

#[derive(Debug, Error)]
pub enum MoveSessionError {
    #[error("session {0} not found under project dir {1:?}")]
    SessionNotFound(Uuid, PathBuf),

    #[error("invalid slug {0:?}: {1}")]
    InvalidSlug(String, &'static str),

    #[error("sync-conflict sibling present for session {0} — resolve manually or pass force_sync_conflict")]
    SyncConflictPresent(Uuid),

    #[error("session {0} appears live (mtime < threshold) — quit Claude Code first or pass force_live_session")]
    LiveSession(Uuid),

    #[error("target slug already contains a file for session {0}")]
    TargetCollision(Uuid),

    #[error("target sidecar path already exists: {0:?} — refusing to overwrite an existing per-session file")]
    SidecarCollision(PathBuf),

    #[error("from_cwd and to_cwd canonicalize to the same path")]
    SameCwd,

    #[error("target cwd {0:?} exists but is not a directory")]
    TargetNotADirectory(PathBuf),

    #[error("{0:?} was truncated or replaced while the move was rewriting it — nothing was written; re-run the move")]
    HistoryFileReplaced(PathBuf),

    #[error("source cwd {0:?} is still a live git worktree of target — CC already handles cross-worktree resume, no move needed")]
    WorktreeSiblingStillLive(PathBuf),

    #[error("config_dir does not exist or is unreadable: {0:?}")]
    InvalidConfigDir(PathBuf),

    #[error("failed to move to Trash: {0}")]
    TrashFailed(String),

    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Hand-written, wildcard-free: a new variant must be named here before
/// it compiles. See `crate::error_code` for the code/params contract.
///
/// Several messages name a `force_*` argument the GUI never shows;
/// those stay frozen in the English text and the code exposes the
/// session id instead, so a localized sentence can name the session and
/// offer a button. Paths go in raw — `Path::display()` only.
impl crate::error_code::ErrorCode for MoveSessionError {
    fn code(&self) -> &'static str {
        match self {
            MoveSessionError::SessionNotFound(_, _) => "session_move.session_not_found",
            MoveSessionError::InvalidSlug(_, _) => "session_move.invalid_slug",
            MoveSessionError::SyncConflictPresent(_) => "session_move.sync_conflict_present",
            MoveSessionError::LiveSession(_) => "session_move.live_session",
            MoveSessionError::TargetCollision(_) => "session_move.target_collision",
            MoveSessionError::SidecarCollision(_) => "session_move.sidecar_collision",
            MoveSessionError::SameCwd => "session_move.same_cwd",
            MoveSessionError::TargetNotADirectory(_) => "session_move.target_not_a_directory",
            MoveSessionError::HistoryFileReplaced(_) => "session_move.history_file_replaced",
            MoveSessionError::WorktreeSiblingStillLive(_) => {
                "session_move.worktree_sibling_still_live"
            }
            MoveSessionError::InvalidConfigDir(_) => "session_move.invalid_config_dir",
            MoveSessionError::TrashFailed(_) => "session_move.trash_failed",
            MoveSessionError::Io(_) => "session_move.io",
        }
    }

    fn params(&self) -> serde_json::Value {
        match self {
            MoveSessionError::SessionNotFound(session_id, path) => serde_json::json!({
                "session_id": session_id.to_string(),
                "path": path.display().to_string(),
            }),
            MoveSessionError::InvalidSlug(slug, reason) => serde_json::json!({
                "slug": slug,
                "reason": reason,
            }),
            MoveSessionError::SyncConflictPresent(session_id) => serde_json::json!({
                "session_id": session_id.to_string(),
            }),
            MoveSessionError::LiveSession(session_id) => serde_json::json!({
                "session_id": session_id.to_string(),
            }),
            MoveSessionError::TargetCollision(session_id) => serde_json::json!({
                "session_id": session_id.to_string(),
            }),
            MoveSessionError::SidecarCollision(path) => serde_json::json!({
                "path": path.display().to_string(),
            }),
            MoveSessionError::SameCwd => serde_json::json!({}),
            MoveSessionError::TargetNotADirectory(path) => serde_json::json!({
                "path": path.display().to_string(),
            }),
            MoveSessionError::HistoryFileReplaced(path) => serde_json::json!({
                "path": path.display().to_string(),
            }),
            MoveSessionError::WorktreeSiblingStillLive(path) => serde_json::json!({
                "path": path.display().to_string(),
            }),
            MoveSessionError::InvalidConfigDir(path) => serde_json::json!({
                "path": path.display().to_string(),
            }),
            MoveSessionError::TrashFailed(detail) => serde_json::json!({ "detail": detail }),
            MoveSessionError::Io(e) => serde_json::json!({ "detail": e.to_string() }),
        }
    }
}

/// A project directory whose internal `cwd` refers to a non-existent
/// directory and has no live-worktree escape hatch. The user's primary
/// cue that a move / adopt is warranted.
#[derive(Debug, Clone)]
pub struct OrphanedProject {
    /// The sanitized directory name under `<config_dir>/projects/`.
    pub slug: String,
    /// Canonical cwd extracted from the first JSONL line in the dir.
    /// `None` if the dir had no parseable JSONL (degenerate orphan).
    pub cwd_from_transcript: Option<PathBuf>,
    pub session_count: usize,
    pub total_size_bytes: u64,
    /// If detectable, a reasonable adoption target — typically the main
    /// worktree of the repo the dead cwd used to be a worktree of, or
    /// the nearest existing ancestor directory.
    pub suggested_adoption_target: Option<PathBuf>,
}

#[derive(Debug, Default, Clone)]
pub struct AdoptReport {
    pub sessions_attempted: usize,
    pub sessions_moved: usize,
    pub sessions_failed: Vec<(Uuid, String)>,
    pub source_dir_removed: bool,
    pub per_session: Vec<MoveSessionReport>,
}

/// Summary of an orphan-project discard. The slug dir is moved to the
/// OS Trash as a whole (macOS ~/.Trash, Windows Recycle Bin, XDG trash).
/// `sessions_discarded` and `total_size_bytes` are snapshotted *before*
/// the trash call so the UI can echo the cost even if the underlying
/// dir no longer exists by the time the report is rendered.
#[derive(Debug, Default, Clone)]
pub struct DiscardReport {
    pub sessions_discarded: usize,
    pub total_size_bytes: u64,
    /// True iff the slug dir is no longer present on disk after the
    /// trash call. On all supported platforms the `trash` crate moves
    /// the directory, so this is normally true; it is `false` only
    /// when the trash operation reports success but leaves a remnant
    /// (shouldn't happen in practice — kept for observability).
    pub dir_removed: bool,
}
