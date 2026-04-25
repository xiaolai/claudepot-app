// Long-running operation DTOs: move args, journal flags, op kinds, verify events, op-progress events.
// Sharded from src/types.ts to keep each domain's DTOs in its own
// file; src/types/index.ts re-exports them. Mirrors src-tauri/src/dto.rs.

import type { CleanResult } from "./project";
import type { MoveSessionReport } from "./session";

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
  | "clean_projects"
  | "session_prune"
  | "session_slim"
  | "session_share"
  | "session_move"
  | "account_login"
  | "account_register"
  | "verify_all";

/** Phase ids emitted by `session_move_with_progress`. Stable contract
 *  with `crates/claudepot-core/src/session_move.rs`. */
export type SessionMovePhase = "S1" | "S2" | "S3" | "S4" | "S5";

/** Discrete steps of the browser-OAuth login pipeline. Stable contract
 *  with `claudepot_core::services::account_service::LoginPhase`. */
export type LoginPhase =
  | "spawning"
  | "waiting_for_browser"
  | "reading_blob"
  | "fetching_profile"
  | "verifying_identity"
  | "persisting";

/** Per-account verify outcome — flat enum mirroring
 *  `claudepot_core::services::account_service::VerifyOutcomeKind`. */
export type VerifyOutcomeKind = "ok" | "drift" | "rejected" | "network_error";

/** Per-account event emitted on `op-progress::<op_id>` for VerifyAll
 *  ops. Sibling payload to `OperationProgressEvent` — distinguished by
 *  its `kind: "verify_account"` discriminator. */
export interface VerifyAccountEvent {
  op_id: string;
  kind: "verify_account";
  uuid: string;
  email: string;
  idx: number;
  total: number;
  outcome: VerifyOutcomeKind;
  detail?: string;
}

/** Counters bundled at the end of a `verify_all` op. */
export interface VerifyResultSummary {
  total: number;
  ok: number;
  drift: number;
  rejected: number;
  network_error: number;
}

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
  /** Populated on successful SessionMove. Same shape as the legacy
   *  `sessionMove` IPC's return value. */
  session_move_result?: MoveSessionReport | null;
  /** Latest login phase observed by the polling backstop. Mirrors
   *  `current_phase` (string) but typed for the GUI. */
  login_phase?: LoginPhase | null;
  /** Aggregate counters for a `verify_all` op. */
  verify_results?: VerifyResultSummary | null;
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

export interface AbandonedCleanupEntry {
  id: string;
  journalPath: string;
  sidecarPath: string;
  referencedSnapshots: string[];
  bytes: number;
}

export interface AbandonedCleanupReport {
  entries: AbandonedCleanupEntry[];
  removedJournals: number;
  removedSnapshots: number;
  bytesFreed: number;
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
