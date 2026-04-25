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
 * Preview of what `project_clean_execute` would delete. The UI
 * renders this in the confirm modal before the user approves the
 * actual run. `unreachable_skipped` surfaces projects whose source
 * lives on an unmounted volume — they are NOT candidates for cleanup
 * and shouldn't be in the list.
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
 * Outcome of a completed `project_clean_execute`. The modal renders
 * every non-zero counter as a line item. `snapshot_paths` points at
 * the recovery snapshots (~/.claude.json entry value, dropped
 * history.jsonl lines) so the user can restore if the clean turned
 * out to be wrong.
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

/**
 * One row in the protected-paths Settings list. `source` drives the
 * badge: `"default"` rows came from the built-in DEFAULT_PATHS;
 * `"user"` rows are user-added.
 */
export interface ProtectedPath {
  path: string;
  source: "default" | "user";
}

/**
 * Persisted UI preferences. Backed by `preferences.json` in the
 * Claudepot data dir; read synchronously at Rust startup.
 */
export interface Preferences {
  /** macOS-only: when true, the app runs tray-only (no dock icon, no
   *  Cmd+Tab, no app menu bar). No-op on Windows/Linux. */
  hide_dock_icon: boolean;

  /** User opted in to the live Activity feature. Gate for starting
   *  the LiveRuntime. Defaults to false until the consent modal is
   *  accepted. */
  activity_enabled: boolean;

  /** First-run consent modal has been seen (accepted OR declined).
   *  Separate from activity_enabled so a user who declined once
   *  isn't re-prompted every launch. */
  activity_consent_seen: boolean;

  /** Thinking blocks render redacted-by-default with a "▸ reveal"
   *  affordance. Defaults to true — privacy-forward. */
  activity_hide_thinking: boolean;

  /** Project paths the live runtime should ignore. Path-prefix
   *  matched against PidRecord.cwd. */
  activity_excluded_paths: string[];

  notify_on_error: boolean;
  notify_on_idle_done: boolean;
  /** null = feature off; number = fire after N minutes stuck. */
  notify_on_stuck_minutes: number | null;
  /** null = feature off; number = fire when session spend >= $. */
  notify_on_spend_usd: number | null;
}

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
