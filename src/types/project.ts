// Project list/detail, clean preview/result, protected paths, preferences, dry-run plan.
// Sharded from src/types.ts to keep each domain's DTOs in its own
// file; src/types/index.ts re-exports them. Mirrors src-tauri/src/dto.rs.

// ---------------------------------------------------------------------------
// Project DTOs — mirror src-tauri/src/dto.rs ProjectInfoDto et al.
// ---------------------------------------------------------------------------

export interface ProjectInfo {
  sanitized_name: string;
  original_path: string;
  session_count: number;
  memory_file_count: number;
  total_size_bytes: number;
  /** ms since epoch (pass to `new Date(ms)`), or null if never modified. */
  last_modified_ms: number | null;
  is_orphan: boolean;
  /**
   * True when we could definitively stat the source path. False for
   * projects whose source lives under an unmounted volume / offline
   * share / permission-denied ancestor — these are NEVER auto-cleaned.
   */
  is_reachable: boolean;
  /** CC project dir has no sessions, no memory, minimal disk footprint. */
  is_empty: boolean;
}

export interface SessionInfo {
  session_id: string;
  file_size: number;
  last_modified_ms: number | null;
}

export interface ProjectDetail {
  info: ProjectInfo;
  sessions: SessionInfo[];
  memory_files: string[];
}

/**
 * Preview of what `projectCleanStart` would delete. Returned by
 * `projectCleanPreview`; the UI renders this in the confirm modal
 * before the user approves the actual run. `unreachable_skipped`
 * surfaces projects whose source lives on an unmounted volume —
 * they are NOT candidates for cleanup and shouldn't be in the list.
 */
export interface CleanPreview {
  orphans: ProjectInfo[];
  orphans_found: number;
  unreachable_skipped: number;
  total_bytes: number;
  /**
   * Count of candidates whose authoritative `original_path` is in the
   * user's protected-paths set. Their CC artifact dir will still be
   * removed; only sibling state (`~/.claude.json`, `history.jsonl`) is
   * preserved for them.
   */
  protected_count: number;
}

/**
 * Outcome of a completed `projectCleanStart` op (delivered via the
 * `op-progress::<op_id>` terminal event and surfaced through
 * `RunningOpInfo.clean_result`). The modal renders every non-zero
 * counter as a line item. `snapshot_paths` points at the recovery
 * snapshots (~/.claude.json entry value, dropped history.jsonl
 * lines) so the user can restore if the clean turned out to be
 * wrong.
 */
export interface CleanResult {
  orphans_found: number;
  orphans_removed: number;
  orphans_skipped_live: number;
  unreachable_skipped: number;
  bytes_freed: number;
  claude_json_entries_removed: number;
  history_lines_removed: number;
  claudepot_artifacts_removed: number;
  snapshot_paths: string[];
  /**
   * Count of orphans whose `original_path` matched the user's
   * protected-paths set. Their CC artifact dirs were removed; sibling
   * state in `~/.claude.json` and `history.jsonl` was left intact.
   */
  protected_paths_skipped: number;
}

// ---------------------------------------------------------------------------
// Project remove + project trash — single-target reversible delete
// ---------------------------------------------------------------------------

/**
 * Cheap subset of `RemoveProjectPreview` — what the modal renders on
 * first paint. Skips the live-session probe and the multi-MB sibling-
 * state reads, so it lands in <50 ms even on prolific users.
 */
export interface RemoveProjectPreviewBasic {
  slug: string;
  original_path: string | null;
  bytes: number;
  session_count: number;
  last_modified_ms: number | null;
}

/**
 * Slow subset — live-session probe + sibling-state counts. Comes in
 * via a follow-up call so the modal stays responsive while these
 * resolve.
 */
export interface RemoveProjectPreviewExtras {
  has_live_session: boolean;
  claude_json_entry_present: boolean;
  history_lines_count: number;
}

/**
 * Read-only preview the RemoveProjectModal renders. Mirrors
 * `RemoveProjectPreviewDto` in src-tauri. `has_live_session` is
 * informational — the modal disables the confirm path with an inline
 * reason when true, per the design rule on disabled buttons stating
 * a reason inline.
 */
export interface RemoveProjectPreview {
  slug: string;
  /** Best-effort recovered cwd. Null when the dir is empty AND no
   *  `.claude.json` key matches the unsanitized slug. */
  original_path: string | null;
  bytes: number;
  session_count: number;
  last_modified_ms: number | null;
  has_live_session: boolean;
  claude_json_entry_present: boolean;
  history_lines_count: number;
}

/** Outcome of a successful `projectRemoveExecute`. */
export interface RemoveProjectResult {
  slug: string;
  original_path: string | null;
  bytes: number;
  session_count: number;
  trash_id: string;
  claude_json_entry_removed: boolean;
  history_lines_removed: number;
}

/** One row in the project Trash drawer. */
export interface ProjectTrashEntry {
  id: string;
  slug: string;
  original_path: string | null;
  bytes: number;
  session_count: number;
  ts_ms: number;
  has_claude_json_entry: boolean;
  history_lines_count: number;
}

export interface ProjectTrashListing {
  entries: ProjectTrashEntry[];
  total_bytes: number;
}

export interface ProjectRestoreReport {
  restored_dir: string;
  claude_json_restored: boolean;
  history_lines_restored: number;
}

// ProtectedPath + Preferences moved to src/types/settings.ts so the
// project shard stays domain-coherent. The index.ts re-export keeps
// `from "../types"` import sites unchanged.

export interface DryRunPlan {
  would_move_dir: boolean;
  old_cc_dir: string;
  new_cc_dir: string;
  session_count: number;
  cc_dir_size: number;
  estimated_history_lines: number;
  /** Non-null when CC dir at target is non-empty without --merge/--overwrite. */
  conflict: string | null;
  estimated_jsonl_files: number;
  would_rewrite_claude_json: boolean;
  would_move_memory_dir: boolean;
  would_rewrite_project_settings: boolean;
}
