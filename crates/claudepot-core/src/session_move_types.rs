//! Types + shared constants for `session_move`.
//!
//! Kept separate per loc-guardian rule 1 (types >30 LOC extract to
//! `<module>_types.rs`).

use std::path::PathBuf;
use std::time::Duration;
use thiserror::Error;
use uuid::Uuid;

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

    #[error("sync-conflict sibling present for session {0} — resolve manually or pass force_sync_conflict")]
    SyncConflictPresent(Uuid),

    #[error("session {0} appears live (mtime < threshold) — quit Claude Code first or pass force_live_session")]
    LiveSession(Uuid),

    #[error("target slug already contains a file for session {0}")]
    TargetCollision(Uuid),

    #[error("from_cwd and to_cwd canonicalize to the same path")]
    SameCwd,

    #[error("source cwd {0:?} is still a live git worktree of target — CC already handles cross-worktree resume, no move needed")]
    WorktreeSiblingStillLive(PathBuf),

    #[error("config_dir does not exist or is unreadable: {0:?}")]
    InvalidConfigDir(PathBuf),

    #[error(transparent)]
    Io(#[from] std::io::Error),
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
