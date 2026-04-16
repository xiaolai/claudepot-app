// Thin wrappers around Tauri's invoke — one function per Rust command.
import { invoke } from "@tauri-apps/api/core";
import type {
  AccountSummary,
  AppStatus,
  BreakLockOutcome,
  CcIdentity,
  CleanPreview,
  CleanResult,
  DryRunPlan,
  GcOutcome,
  JournalEntry,
  MoveArgs,
  PendingJournalsSummary,
  ProjectDetail,
  ProjectInfo,
  RegisterOutcome,
  RemoveOutcome,
  RunningOpInfo,
  UsageMap,
} from "./types";

export const api = {
  appStatus: () => invoke<AppStatus>("app_status"),
  /// Idempotent startup adoption: if CC holds credentials for one of the
  /// registered accounts, imports them into the matching slot. Returns
  /// the synced email (empty string when nothing matched).
  syncFromCurrentCc: () => invoke<string>("sync_from_current_cc"),
  /// macOS-only: request a native keychain-unlock dialog. The user's
  /// password is entered directly into macOS's own trusted prompt and
  /// never reaches Claudepot.
  unlockKeychain: () => invoke<void>("unlock_keychain"),
  accountList: () => invoke<AccountSummary[]>("account_list"),
  cliUse: (email: string) => invoke<void>("cli_use", { email }),
  cliClear: () => invoke<void>("cli_clear"),
  desktopUse: (email: string, noLaunch: boolean) =>
    invoke<void>("desktop_use", { email, noLaunch }),
  accountAddFromCurrent: () =>
    invoke<RegisterOutcome>("account_add_from_current"),
  // Token-based onboarding is CLI-only — the refresh token must never enter
  // the webview JS heap. Use a future browser-flow command instead.
  /// Re-log in via browser (opens Claude's OAuth flow) and imports the
  /// resulting blob into the given account's slot. Can take several
  /// minutes while the user completes auth in the browser.
  accountLogin: (uuid: string) => invoke<void>("account_login", { uuid }),
  accountLoginCancel: () => invoke<void>("account_login_cancel"),
  accountRemove: (uuid: string) =>
    invoke<RemoveOutcome>("account_remove", { uuid }),
  fetchAllUsage: () => invoke<UsageMap>("fetch_all_usage"),
  /// Reconcile every account's blob identity against `/api/oauth/profile`.
  /// Returns the refreshed list so the caller can re-render without a
  /// separate `accountList` round-trip. Slow — one HTTP call per account
  /// with credentials.
  verifyAllAccounts: () => invoke<AccountSummary[]>("verify_all_accounts"),
  /// Ground-truth "what is CC currently authenticated as". Reads the
  /// shared slot + calls /profile. Never throws — errors land in the
  /// returned `error` field so the UI can render them as a banner.
  currentCcIdentity: () => invoke<CcIdentity>("current_cc_identity"),

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
   * Read-only preview of `projectCleanExecute`. Lists orphan CC project
   * dirs whose source is confirmed absent, plus a count of unreachable
   * candidates (unmounted volume / permission denied) that will NOT
   * be cleaned. Safe to call on open of the confirm modal.
   */
  projectCleanPreview: () => invoke<CleanPreview>("project_clean_preview"),
  /**
   * Irreversible. Deletes every orphan CC project dir, purges
   * matching `~/.claude.json` entries + `history.jsonl` lines (with
   * recovery snapshots), and reclaims stale claudepot-owned
   * artifacts. Gated on no pending rename journals.
   */
  projectCleanExecute: () => invoke<CleanResult>("project_clean_execute"),

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

  // ---------- Op tracking ----------
  /** Snapshot of currently-tracked ops. Backstop for event drops. */
  runningOpsList: () => invoke<RunningOpInfo[]>("running_ops_list"),
};
