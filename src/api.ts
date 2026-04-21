// Thin wrappers around Tauri's invoke — one function per Rust command.
import { invoke } from "@tauri-apps/api/core";
import type {
  AccountSummary,
  AdoptReport,
  AppStatus,
  BreakLockOutcome,
  CcIdentity,
  CleanPreview,
  DryRunPlan,
  GcOutcome,
  JournalEntry,
  MoveArgs,
  MoveSessionReport,
  OrphanedProject,
  PendingJournalsSummary,
  ProjectDetail,
  ProjectInfo,
  Preferences,
  ProtectedPath,
  RegisterOutcome,
  RemoveOutcome,
  RunningOpInfo,
  ContextStats,
  ContextPhaseInfo,
  LinkedTool,
  RepositoryGroup,
  SearchHit,
  SessionChunk,
  SessionDetail,
  SessionRow,
  Subagent,
  UsageEntry,
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
  /// Reveal a path in the native file manager (Finder / Explorer /
  /// file manager). Walks up to the nearest existing parent if the
  /// exact path is gone (orphan projects still "open parent").
  revealInFinder: (path: string) => invoke<void>("reveal_in_finder", { path }),
  accountList: () => invoke<AccountSummary[]>("account_list"),
  cliUse: (email: string, force = false) =>
    invoke<void>("cli_use", { email, force }),
  /// Cheap preflight used before cli_use to decide whether to raise
  /// the split-brain confirmation dialog.
  cliIsCcRunning: () => invoke<boolean>("cli_is_cc_running"),
  cliClear: () => invoke<void>("cli_clear"),
  desktopUse: (email: string, noLaunch: boolean) =>
    invoke<void>("desktop_use", { email, noLaunch }),
  accountAddFromCurrent: () =>
    invoke<RegisterOutcome>("account_add_from_current"),
  /// Browser OAuth onboarding — spawns `claude auth login` in a temp
  /// config dir, returns when the user finishes (or errors). The
  /// refresh token never crosses the IPC bridge; everything is handled
  /// by claudepot-core on the Rust side.
  accountRegisterFromBrowser: () =>
    invoke<RegisterOutcome>("account_register_from_browser"),
  // Token-based onboarding is CLI-only — the refresh token must never enter
  // the webview JS heap. Browser onboarding above is the GUI equivalent.
  /// Re-log in via browser (opens Claude's OAuth flow) and imports the
  /// resulting blob into the given account's slot. Can take several
  /// minutes while the user completes auth in the browser.
  accountLogin: (uuid: string) => invoke<void>("account_login", { uuid }),
  accountLoginCancel: () => invoke<void>("account_login_cancel"),
  accountRemove: (uuid: string) =>
    invoke<RemoveOutcome>("account_remove", { uuid }),
  fetchAllUsage: () => invoke<UsageMap>("fetch_all_usage"),
  /// Invalidate cache + cooldown for a single account then refetch.
  /// Scoped alternative to fetchAllUsage for per-row Retry buttons.
  refreshUsageFor: (uuid: string) =>
    invoke<UsageEntry>("refresh_usage_for", { uuid }),
  /// Reconcile every account's blob identity against `/api/oauth/profile`.
  /// Returns the refreshed list so the caller can re-render without a
  /// separate `accountList` round-trip. Slow — one HTTP call per account
  /// with credentials.
  verifyAllAccounts: () => invoke<AccountSummary[]>("verify_all_accounts"),
  /// Verify a single account — fast, single /profile round-trip. Used
  /// by the per-row context menu and command palette.
  verifyAccount: (uuid: string) =>
    invoke<AccountSummary>("verify_account", { uuid }),
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

  // ---------- Session move ----------
  /**
   * Scan ~/.claude/projects for slugs whose internal cwd no longer
   * exists on disk. Returns the set of adoption candidates — the
   * primary surface of the orphan-rescue flow.
   */
  sessionListOrphans: () => invoke<OrphanedProject[]>("session_list_orphans"),
  /**
   * Move a single session transcript from one project cwd to another.
   * Surfaces touched: primary JSONL (cwd rewrite every line), the
   * session's subagents/remote-agents subdir, history.jsonl entries
   * keyed by sessionId, and .claude.json's lastSessionId /
   * activeWorktreeSession.sessionId pointers for the source cwd.
   */
  sessionMove: (args: {
    sessionId: string;
    fromCwd: string;
    toCwd: string;
    forceLive?: boolean;
    forceConflict?: boolean;
    cleanupSource?: boolean;
  }) =>
    invoke<MoveSessionReport>("session_move", {
      sessionId: args.sessionId,
      fromCwd: args.fromCwd,
      toCwd: args.toCwd,
      forceLive: args.forceLive ?? false,
      forceConflict: args.forceConflict ?? false,
      cleanupSource: args.cleanupSource ?? false,
    }),
  /**
   * Move every session under an orphaned slug into a live target cwd.
   * Force-bypasses the live-mtime guard since an orphan's cwd is gone
   * by definition.
   */
  sessionAdoptOrphan: (slug: string, targetCwd: string) =>
    invoke<AdoptReport>("session_adopt_orphan", { slug, targetCwd }),

  // ---------- Session index (Sessions tab) ----------
  /**
   * Walk every `~/.claude/projects/<slug>/<session>.jsonl` and produce
   * list rows with token totals, models seen, first-prompt preview,
   * and CC version. Newest-first by the last-event timestamp (falling
   * back to file mtime). Backed by a persistent SQLite cache in
   * `~/.claudepot/sessions.db` — cold first call folds every
   * transcript; subsequent calls touch only `stat()` and the delta.
   */
  sessionListAll: () => invoke<SessionRow[]>("session_list_all"),
  /**
   * Truncate the session-index cache and force the next `sessionListAll`
   * to re-parse every transcript from cold. Escape hatch for cases the
   * `(size, mtime)` guard can't see. Safe to call — no data loss; only
   * derived cache rows are dropped.
   */
  sessionIndexRebuild: () => invoke<void>("session_index_rebuild"),
  /**
   * Full transcript + row metadata for one session, keyed by its
   * UUID. Locates the slug by filename match, then streams the JSONL
   * into normalized `SessionEvent`s.
   */
  sessionRead: (sessionId: string) =>
    invoke<SessionDetail>("session_read", { sessionId }),
  /**
   * Preferred over `sessionRead` from the Sessions tab — reading by
   * path disambiguates the rare case where two .jsonl files share one
   * session_id (interrupted adopt/rescue). Path must live under
   * `<config>/projects/`.
   */
  sessionReadPath: (filePath: string) =>
    invoke<SessionDetail>("session_read_path", { filePath }),

  // ---------- Session debugger (Tier 1-3 port from claude-devtools) ----------
  /** Chunked event stream (User/Ai/System/Compact) with per-chunk linked tools. */
  sessionChunks: (filePath: string) =>
    invoke<SessionChunk[]>("session_chunks", { filePath }),
  /** Paired tool calls and results for one transcript. */
  sessionLinkedTools: (filePath: string) =>
    invoke<LinkedTool[]>("session_linked_tools", { filePath }),
  /** Subagent transcripts attached to a parent session. */
  sessionSubagents: (filePath: string) =>
    invoke<Subagent[]>("session_subagents", { filePath }),
  /** Compaction phase breakdown. One phase per context window. */
  sessionPhases: (filePath: string) =>
    invoke<ContextPhaseInfo>("session_phases", { filePath }),
  /** Visible-context token attribution across six categories. */
  sessionContextAttribution: (filePath: string) =>
    invoke<ContextStats>("session_context_attribution", { filePath }),
  /** Export transcript as Markdown or JSON (sk-ant-* redacted). */
  sessionExportText: (filePath: string, format: "md" | "json") =>
    invoke<string>("session_export_text", { filePath, format }),
  /** Export transcript and write to disk (0600 on Unix). Returns bytes written. */
  sessionExportToFile: (
    filePath: string,
    format: "md" | "json",
    outputPath: string,
  ) =>
    invoke<number>("session_export_to_file", {
      filePath,
      format,
      outputPath,
    }),
  /** Cross-session text search. Returns ranked hits. */
  sessionSearch: (query: string, limit = 25) =>
    invoke<SearchHit[]>("session_search", { query, limit }),
  /** Group all sessions by git repository (collapses worktrees). */
  sessionWorktreeGroups: () =>
    invoke<RepositoryGroup[]>("session_worktree_groups"),

  // ---------- Protected paths (Settings → Protected pane) ----------
  /**
   * Materialized list — defaults (minus removed_defaults) followed by
   * user-added entries in insertion order. Order is stable so the UI
   * can render without sorting.
   */
  protectedPathsList: () => invoke<ProtectedPath[]>("protected_paths_list"),
  /**
   * Add a path. Validates and persists. Returns the new entry; if the
   * path matches a previously-removed default, the entry comes back
   * with `source: "default"` (un-tombstoned, not duplicated under user).
   */
  protectedPathsAdd: (path: string) =>
    invoke<ProtectedPath>("protected_paths_add", { path }),
  /**
   * Remove a path. Defaults are tombstoned (so reset() brings them
   * back); user entries are dropped.
   */
  protectedPathsRemove: (path: string) =>
    invoke<void>("protected_paths_remove", { path }),
  /** Restore the implicit defaults; returns the resulting list. */
  protectedPathsReset: () => invoke<ProtectedPath[]>("protected_paths_reset"),

  // ---------- Preferences (Settings → General) ----------
  /** Read the current persisted UI preferences. */
  preferencesGet: () => invoke<Preferences>("preferences_get"),
  /**
   * Toggle hide-dock-icon. Applies `set_activation_policy` immediately
   * on macOS (Accessory = tray-only; Regular = dock + menu bar), then
   * persists the boolean. No-op on Windows/Linux.
   */
  preferencesSetHideDockIcon: (hide: boolean) =>
    invoke<void>("preferences_set_hide_dock_icon", { hide }),
};
