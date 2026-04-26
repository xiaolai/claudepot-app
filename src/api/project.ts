// Project list/detail + clean + repair + running-ops.
// Sharded from src/api.ts; src/api/index.ts merges every
// domain slice into the canonical `api` object.

import { invoke } from "@tauri-apps/api/core";
import type {
  AbandonedCleanupReport,
  BreakLockOutcome,
  CleanPreview,
  DryRunPlan,
  GcOutcome,
  JournalEntry,
  MoveArgs,
  PendingJournalsSummary,
  ProjectDetail,
  ProjectInfo,
  ProjectRestoreReport,
  ProjectTrashListing,
  RemoveProjectPreview,
  RemoveProjectPreviewBasic,
  RemoveProjectPreviewExtras,
  RemoveProjectResult,
  RunningOpInfo,
} from "../types";

export const projectApi = {
  // ---------- Projects (read-only) ----------
  /** List every CC project dir with size, session count, orphan flag. */
  projectList: () => invoke<ProjectInfo[]>("project_list"),
  /** Detail for a single project by original path (pre-sanitization). */
  projectShow: (path: string) => invoke<ProjectDetail>("project_show", { path }),
  /**
   * Compute what a prospective rename would do, without touching disk.
   * Debounce keystrokes before calling on long paths (spec §7.1).
   */
  projectMoveDryRun: (args: MoveArgs) =>
    invoke<DryRunPlan>("project_move_dry_run", { args }),
  /**
   * Kick off an actual rename in the background. Returns the op_id;
   * subscribe to `op-progress::<op_id>` for phase events and call
   * `projectMoveStatus(opId)` to fetch the structured result once
   * the terminal event fires.
   */
  projectMoveStart: (args: MoveArgs) =>
    invoke<string>("project_move_start", { args }),
  /** Poll current state of an in-flight move. null if op_id unknown. */
  projectMoveStatus: (opId: string) =>
    invoke<RunningOpInfo | null>("project_move_status", { opId }),
  /**
   * Read-only preview of `projectCleanStart`. Lists orphan CC project
   * dirs whose source is confirmed absent, plus a count of unreachable
   * candidates (unmounted volume / permission denied) that will NOT
   * be cleaned. Safe to call on open of the confirm modal.
   */
  projectCleanPreview: () => invoke<CleanPreview>("project_clean_preview"),
  /**
   * Irreversible. Kicks off the actual clean in the background,
   * returning an op_id that the UI subscribes to on
   * `op-progress::<op_id>` for phase + sub_progress events. Gated on
   * no pending rename journals. Clean is async so large batches
   * (hundreds of dirs, multi-GB) emit live progress instead of
   * blocking the IPC worker.
   */
  projectCleanStart: () => invoke<string>("project_clean_start"),
  /**
   * Poll the current state of an in-flight clean. Used as a backstop
   * in case an op-progress event drops — the modal reads the final
   * `clean_result` from here once the terminal event fires.
   */
  projectCleanStatus: (opId: string) =>
    invoke<RunningOpInfo | null>("project_clean_status", { opId }),

  // ---------- Project remove + trash ----------
  /**
   * Cheap preview — fields the modal needs to render the disclosure on
   * first paint. Returns in <50 ms even when sibling state is multi-MB.
   * Pair with `projectRemovePreviewExtras` for the slow-probe data.
   */
  projectRemovePreviewBasic: (target: string) =>
    invoke<RemoveProjectPreviewBasic>("project_remove_preview_basic", { target }),
  /**
   * Slow preview — live-session probe + sibling-state counts. Issued
   * in parallel with the basic call so the modal renders without
   * waiting on it.
   */
  projectRemovePreviewExtras: (target: string) =>
    invoke<RemoveProjectPreviewExtras>("project_remove_preview_extras", {
      target,
    }),
  /**
   * Combined preview — used by the CLI / non-interactive callers that
   * want every field in one round trip. The GUI prefers the
   * basic+extras split for responsiveness.
   */
  projectRemovePreview: (target: string) =>
    invoke<RemoveProjectPreview>("project_remove_preview", { target }),
  /**
   * Trashes the project's CC artifact dir, snapshots and prunes its
   * `.claude.json` entry + matching history.jsonl lines. Reversible
   * via `projectTrashRestore` until the trash GC sweeps it (default
   * 30 days). Synchronous from the frontend's perspective — typical
   * removes complete in <1 s.
   */
  projectRemoveExecute: (target: string) =>
    invoke<RemoveProjectResult>("project_remove_execute", { target }),
  /** Newest-first list of trashed projects with sibling-state hints. */
  projectTrashList: () => invoke<ProjectTrashListing>("project_trash_list"),
  /**
   * Restore a trashed project. Refuses to clobber if the user has
   * since recreated a project at the same slug.
   */
  projectTrashRestore: (entryId: string) =>
    invoke<ProjectRestoreReport>("project_trash_restore", { entryId }),
  /**
   * Permanently delete trashed projects. Irreversible. `olderThanDays`
   * filters; null means everything matches.
   */
  projectTrashEmpty: (olderThanDays: number | null) =>
    invoke<number>("project_trash_empty", { olderThanDays }),

  // ---------- Repair (read-only) ----------
  /** Every journal on disk with its classified status. Includes abandoned. */
  repairList: () => invoke<JournalEntry[]>("repair_list"),
  /** Count of *actionable* journals — for the pending-journals banner. */
  repairPendingCount: () => invoke<number>("repair_pending_count"),
  /**
   * Per-status counts of pending journals. Banner uses this to pick
   * tone (neutral for pending, warning for stale); running entries
   * are surfaced separately so the banner can suppress itself for them.
   */
  repairStatusSummary: () =>
    invoke<PendingJournalsSummary>("repair_status_summary"),

  // ---------- Repair (mutating) ----------
  /**
   * Kick off a resume in the background. Returns the op_id — caller
   * subscribes to `op-progress::<op_id>` for phase events.
   */
  repairResumeStart: (id: string) => invoke<string>("repair_resume_start", { id }),
  /** Kick off a rollback in the background. Returns op_id. */
  repairRollbackStart: (id: string) => invoke<string>("repair_rollback_start", { id }),
  /** Write the .abandoned.json sidecar. Synchronous; no events. */
  repairAbandon: (id: string) => invoke<void>("repair_abandon", { id }),
  /** Force-break a lock file (with audit). Synchronous. */
  repairBreakLock: (path: string) =>
    invoke<BreakLockOutcome>("repair_break_lock", { path }),
  /** GC abandoned journals + old snapshots. dryRun=true reports only. */
  repairGc: (olderThanDays: number, dryRun: boolean) =>
    invoke<GcOutcome>("repair_gc", { olderThanDays, dryRun }),
  /** Preview the list of abandoned journals and their referenced
   *  snapshots — non-destructive. Use this before offering Clean. */
  repairPreviewAbandoned: () =>
    invoke<AbandonedCleanupReport>("repair_preview_abandoned"),
  /** Remove every abandoned journal + sidecar + its referenced
   *  snapshots. Safer than `repairGc(0, ...)`: only touches files
   *  linked to an abandoned entry — unreferenced or recent
   *  snapshots from successful ops are left alone. */
  repairCleanupAbandoned: () =>
    invoke<AbandonedCleanupReport>("repair_cleanup_abandoned"),

  // ---------- Op tracking ----------
  /** Snapshot of currently-tracked ops. Backstop for event drops. */
  runningOpsList: () => invoke<RunningOpInfo[]>("running_ops_list"),

};
