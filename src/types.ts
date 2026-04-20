// Shape of DTOs returned by the Rust side. Keep in sync with src-tauri/src/dto.rs.

export interface AccountSummary {
  uuid: string;
  email: string;
  org_name: string | null;
  subscription_type: string | null;
  is_cli_active: boolean;
  is_desktop_active: boolean;
  has_cli_credentials: boolean;
  has_desktop_profile: boolean;
  last_cli_switch: string | null; // RFC3339
  last_desktop_switch: string | null;
  token_status: string; // "valid (...)", "expired", "no credentials", ...
  token_remaining_mins: number | null;
  credentials_healthy: boolean; // true iff stored blob exists + parses
  /** "never" | "ok" | "drift" | "rejected" | "network_error" */
  verify_status: string;
  /** Server-observed email for this slot (may differ from `email` → drift). */
  verified_email: string | null;
  verified_at: string | null; // RFC3339
  /** True iff verified_email differs from email — misfiled slot. */
  drift: boolean;
}

/**
 * Ground-truth "what is CC actually authenticated as" — the UI renders
 * this directly in the top-of-window truth strip. Equivalent of running
 * `claude auth status`.
 */
export interface CcIdentity {
  /** Email /api/oauth/profile returned, or null if CC has no blob. */
  email: string | null;
  /** RFC3339 timestamp of when we ran the profile check. */
  verified_at: string;
  /** Populated when CC has a blob but /profile failed. */
  error: string | null;
}

export interface AppStatus {
  platform: string; // "macos" | "linux" | "windows"
  arch: string;
  cli_active_email: string | null;
  desktop_active_email: string | null;
  desktop_installed: boolean;
  data_dir: string;
  account_count: number;
}

export interface RegisterOutcome {
  email: string;
  org_name: string;
  subscription_type: string;
}

export interface RemoveOutcome {
  email: string;
  was_cli_active: boolean;
  was_desktop_active: boolean;
  had_desktop_profile: boolean;
  warnings: string[];
}

export interface UsageWindow {
  utilization: number; // 0–100
  /** RFC3339, or null when the window has no reset timestamp yet. */
  resets_at: string | null;
}

export interface ExtraUsage {
  is_enabled: boolean;
  monthly_limit: number | null;
  used_credits: number | null;
  /** Server-computed utilization percent — prefer over used/limit ratio. */
  utilization: number | null;
}

export interface AccountUsage {
  five_hour: UsageWindow | null;
  seven_day: UsageWindow | null;
  seven_day_opus: UsageWindow | null;
  seven_day_sonnet: UsageWindow | null;
  /** Third-party OAuth-app usage against this account (render-if-nonzero). */
  seven_day_oauth_apps: UsageWindow | null;
  /** Cowork / shared-seat usage pool (render-if-nonzero). */
  seven_day_cowork: UsageWindow | null;
  extra_usage: ExtraUsage | null;
}

/**
 * Per-account usage entry. Carries an explicit `status` so the UI can
 * render an inline explanation when data is unavailable, instead of
 * the old "silently omit the row" behavior.
 *
 * Status values:
 *   - "ok"              — fresh data (use `usage`)
 *   - "stale"           — cached data, see `age_secs` for staleness
 *   - "no_credentials"  — account has no blob (rare; filtered upstream)
 *   - "expired"         — token past local expiry → prompt re-login
 *   - "rate_limited"    — cooldown, see `retry_after_secs`
 *   - "error"           — other failure, see `error_detail`
 */
export interface UsageEntry {
  status:
    | "ok"
    | "stale"
    | "no_credentials"
    | "expired"
    | "rate_limited"
    | "error";
  usage: AccountUsage | null;
  age_secs: number | null;
  retry_after_secs: number | null;
  error_detail: string | null;
}

/** UUID string → usage entry. Every account with credentials appears
 *  here; the entry's `status` tells the UI whether to render data or
 *  an inline placeholder. */
export type UsageMap = Record<string, UsageEntry>;

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

/** Args for the dry-run / rename commands. Serialized camelCase. */
export interface MoveArgs {
  oldPath: string;
  newPath: string;
  noMove?: boolean;
  merge?: boolean;
  overwrite?: boolean;
  force?: boolean;
  ignorePendingJournals?: boolean;
  /**
   * Monotonic token for dry-run cancellation. Increment on every
   * input change; the backend drops stale calls when a newer token
   * arrives. Ignored by `project_move_start`.
   */
  cancelToken?: number;
}

/** Per-status journal counts surfaced by `repair_status_summary`. */
export interface PendingJournalsSummary {
  pending: number;
  stale: number;
  running: number;
}

/**
 * Sentinel error string backend returns when a dry-run was superseded
 * by a newer call (client is expected to silently discard the result).
 */
export const DRY_RUN_SUPERSEDED = "__claudepot_dry_run_superseded__";

export interface JournalFlags {
  merge: boolean;
  overwrite: boolean;
  force: boolean;
  no_move: boolean;
}

/** One of "running" | "pending" | "stale" | "abandoned". */
export type JournalStatus = "running" | "pending" | "stale" | "abandoned";

/** Kind of long-running op currently tracked by the backend. */
export type OpKind =
  | "repair_resume"
  | "repair_rollback"
  | "move_project"
  | "clean_projects";

export type OpStatus = "running" | "complete" | "error";

/** Populated on successful terminal events; null while running / on error. */
export interface MoveResultSummary {
  actual_dir_moved: boolean;
  cc_dir_renamed: boolean;
  jsonl_files_scanned: number;
  jsonl_files_modified: number;
  config_had_collision: boolean;
  config_snapshot_path: string | null;
  memory_dir_moved: boolean;
  warnings: string[];
}

/** Snapshot returned by `running_ops_list` / `project_move_status`. */
export interface RunningOpInfo {
  op_id: string;
  kind: OpKind;
  old_path: string;
  new_path: string;
  current_phase: string | null;
  /** Tuple [done, total] when a phase reports sub-progress. */
  sub_progress: [number, number] | null;
  status: OpStatus;
  started_unix_secs: number;
  last_error: string | null;
  move_result: MoveResultSummary | null;
  /** Populated on successful CleanProjects. Mirrors `CleanResult`. */
  clean_result: CleanResult | null;
  /** Journal id of a failed move so the UI can deep-link to Repair. */
  failed_journal_id: string | null;
}

/** Event payload on `op-progress::<op_id>` channels. */
export interface OperationProgressEvent {
  op_id: string;
  /** "P3".."P9" for per-phase events; "op" for the terminal event. */
  phase: string;
  /** "running" | "complete" | "error" */
  status: "running" | "complete" | "error";
  done?: number;
  total?: number;
  detail?: string;
}

export interface BreakLockOutcome {
  prior_pid: number;
  prior_hostname: string;
  prior_started: string;
  audit_path: string;
}

export interface GcOutcome {
  removed_journals: number;
  removed_snapshots: number;
  bytes_freed: number;
  would_remove: string[];
}

export interface JournalEntry {
  id: string;
  path: string;
  status: JournalStatus;
  old_path: string;
  new_path: string;
  started_at: string;
  started_unix_secs: number;
  phases_completed: string[];
  snapshot_paths: string[];
  last_error: string | null;
  flags: JournalFlags;
}

// ---------- Session move ----------

export interface OrphanedProject {
  slug: string;
  cwdFromTranscript: string | null;
  sessionCount: number;
  totalSizeBytes: number;
  suggestedAdoptionTarget: string | null;
}

export interface MoveSessionReport {
  sessionId: string | null;
  fromSlug: string;
  toSlug: string;
  jsonlLinesRewritten: number;
  subagentFilesMoved: number;
  remoteAgentFilesMoved: number;
  historyEntriesMoved: number;
  historyEntriesUnmapped: number;
  claudeJsonPointersCleared: number;
  sourceDirRemoved: boolean;
}

export interface AdoptFailure {
  sessionId: string;
  error: string;
}

export interface AdoptReport {
  sessionsAttempted: number;
  sessionsMoved: number;
  sessionsFailed: AdoptFailure[];
  sourceDirRemoved: boolean;
  perSession: MoveSessionReport[];
}
