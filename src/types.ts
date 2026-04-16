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
}

export interface AccountUsage {
  five_hour: UsageWindow | null;
  seven_day: UsageWindow | null;
  seven_day_opus: UsageWindow | null;
  seven_day_sonnet: UsageWindow | null;
  extra_usage: ExtraUsage | null;
}

/** UUID string → usage data. Missing keys = no data available. */
export type UsageMap = Record<string, AccountUsage>;

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
export type OpKind = "repair_resume" | "repair_rollback" | "move_project";

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
