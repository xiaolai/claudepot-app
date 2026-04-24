// Thin wrappers around Tauri's invoke — one function per Rust command.
import { invoke } from "@tauri-apps/api/core";
import type {
  AccountSummary,
  AccountSummaryBasic,
  AccountUsage,
  ActivityTrends,
  AbandonedCleanupReport,
  AdoptReport,
  DiscardReport,
  ApiKeySummary,
  AppStatus,
  BreakLockOutcome,
  CcIdentity,
  CleanPreview,
  DesktopAdoptOutcome,
  DesktopClearOutcome,
  DesktopIdentity,
  DesktopReconcileOutcome,
  DesktopSyncOutcome,
  DryRunPlan,
  GcOutcome,
  JournalEntry,
  LiveSessionSummary,
  MoveArgs,
  MoveSessionReport,
  OauthTokenSummary,
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
  RepositoryGroup,
  SearchHit,
  SessionChunk,
  SessionDetail,
  SessionRow,
  UsageEntry,
  UsageMap,
  PruneFilterInput,
  PrunePlan,
  SlimOptsInput,
  SlimPlan,
  BulkSlimPlan,
  TrashListing,
  ExportFormatInput,
  RedactionPolicyInput,
  GithubTokenStatus,
  ConfigTreeDto,
  ConfigPreviewDto,
  ConfigKind,
  ConfigEffectiveSettingsDto,
  ConfigEffectiveMcpDto,
  EditorCandidateDto,
  EditorDefaultsDto,
  McpSimulationMode,
  PriceTableDto,
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
  /** Keychain-free lean list — use when you only need identity
   *  fields (uuid / email / org / subscription / active flags).
   *  Much faster than `accountList` because it skips the per-account
   *  `token_health` call that hits macOS Keychain. */
  accountListBasic: () =>
    invoke<AccountSummaryBasic[]>("account_list_basic"),
  cliUse: (email: string, force = false) =>
    invoke<void>("cli_use", { email, force }),
  /// Cheap preflight used before cli_use to decide whether to raise
  /// the split-brain confirmation dialog.
  cliIsCcRunning: () => invoke<boolean>("cli_is_cc_running"),
  cliClear: () => invoke<void>("cli_clear"),
  desktopUse: (email: string, noLaunch: boolean) =>
    invoke<void>("desktop_use", { email, noLaunch }),
  /// Ground-truth "who is Claude Desktop signed in as". Phase 1
  /// returns `org_uuid_candidate` (NOT verified) or `none`; the
  /// `decrypted` trust tier lands in Phase 2. UI must gate mutating
  /// affordances on `probe_method === "decrypted"`.
  currentDesktopIdentity: () =>
    invoke<DesktopIdentity>("current_desktop_identity"),
  /// Strict Desktop identity probe — runs the async Decrypted path so
  /// callers that mutate disk or DB (Bind, switch) get a verified
  /// email. Returns `probe_method === "decrypted"` on success, or
  /// `"none"` with an `error` message on probe failure. Prefer this
  /// over `currentDesktopIdentity` anywhere you gate mutation on
  /// identity — the fast path returns `org_uuid_candidate` only.
  verifiedDesktopIdentity: () =>
    invoke<DesktopIdentity>("verified_desktop_identity"),
  /// Explicit reconcile: flip `has_desktop_profile` to match on-disk
  /// truth + clear orphan `state.active_desktop` pointer. The same
  /// logic runs opportunistically inside `accountList` — this
  /// command surfaces the outcome for "Reconcile now" affordances.
  desktopReconcile: () =>
    invoke<DesktopReconcileOutcome>("desktop_reconcile"),
  /// Adopt the live Desktop session into `uuid`'s snapshot directory.
  /// Always verifies identity via the authoritative Decrypted path
  /// before mutating — fast-path candidates cannot drive adoption.
  desktopAdopt: (uuid: string, overwrite: boolean) =>
    invoke<DesktopAdoptOutcome>("desktop_adopt", { uuid, overwrite }),
  /// Sign Desktop out. `keepSnapshot=true` (default) preserves the
  /// current session as a snapshot under the active account.
  desktopClear: (keepSnapshot: boolean) =>
    invoke<DesktopClearOutcome>("desktop_clear", { keepSnapshot }),
  /// Startup / window-focus sync. Read-only (no disk mutation); at
  /// most refreshes the active_desktop pointer cache.
  syncFromCurrentDesktop: () =>
    invoke<DesktopSyncOutcome>("sync_from_current_desktop"),
  desktopIsRunning: () => invoke<boolean>("desktop_is_running"),
  desktopLaunch: () => invoke<void>("desktop_launch"),
  desktopQuit: () => invoke<void>("desktop_quit"),
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

  /**
   * Move an orphan project slug dir to the OS Trash (reversible).
   * Pair with ConfirmDialog on the caller side — this is destructive
   * from the user's perspective even though Trash makes it recoverable.
   */
  sessionDiscardOrphan: (slug: string) =>
    invoke<DiscardReport>("session_discard_orphan", { slug }),

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
  /** Visible-context token attribution across six categories. */
  sessionContextAttribution: (filePath: string) =>
    invoke<ContextStats>("session_context_attribution", { filePath }),
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

  // ---------- Prune / slim / trash ----------
  /** Preview which sessions match a prune filter. Pure — no disk writes. */
  sessionPrunePlan: (filter: PruneFilterInput) =>
    invoke<PrunePlan>("session_prune_plan", { filter }),
  /** Start an async prune op; returns an op_id to subscribe on. */
  sessionPruneStart: (filter: PruneFilterInput) =>
    invoke<string>("session_prune_start", { filter }),
  /** Preview a slim rewrite without touching disk. */
  sessionSlimPlan: (path: string, opts: SlimOptsInput) =>
    invoke<SlimPlan>("session_slim_plan", { path, opts }),
  /** Start an async slim op; returns an op_id to subscribe on. */
  sessionSlimStart: (path: string, opts: SlimOptsInput) =>
    invoke<string>("session_slim_start", { path, opts }),
  /** Preview a bulk slim across every session matching the filter. */
  sessionSlimPlanAll: (filter: PruneFilterInput, opts: SlimOptsInput) =>
    invoke<BulkSlimPlan>("session_slim_plan_all", { filter, opts }),
  /** Start an async bulk slim op; returns an op_id to subscribe on. */
  sessionSlimStartAll: (filter: PruneFilterInput, opts: SlimOptsInput) =>
    invoke<string>("session_slim_start_all", { filter, opts }),
  /** List trash batches. */
  sessionTrashList: (olderThanSecs?: number) =>
    invoke<TrashListing>("session_trash_list", {
      olderThanSecs: olderThanSecs ?? null,
    }),
  /** Restore a trashed batch to its original cwd or an override. */
  sessionTrashRestore: (entryId: string, overrideCwd?: string) =>
    invoke<string>("session_trash_restore", {
      entryId,
      overrideCwd: overrideCwd ?? null,
    }),
  /** Permanently delete trash batches. Returns bytes freed. */
  sessionTrashEmpty: (olderThanSecs?: number) =>
    invoke<number>("session_trash_empty", {
      olderThanSecs: olderThanSecs ?? null,
    }),

  // ---------- Export preview / share (Phase 3) ----------
  /** Pure preview — identical to what a file export would write. */
  sessionExportPreview: (
    target: string,
    format: ExportFormatInput,
    policy?: RedactionPolicyInput,
  ) =>
    invoke<string>("session_export_preview", {
      target,
      format,
      policy: policy ?? null,
    }),
  /** Start a gist upload; returns an op_id. */
  sessionShareGistStart: (
    target: string,
    format: ExportFormatInput,
    policy: RedactionPolicyInput | undefined,
    isPublic: boolean,
  ) =>
    invoke<string>("session_share_gist_start", {
      target,
      format,
      policy: policy ?? null,
      public: isPublic,
    }),
  /** GitHub PAT management — last4 is the only value that ever crosses. */
  settingsGithubTokenGet: () =>
    invoke<GithubTokenStatus>("settings_github_token_get"),
  settingsGithubTokenSet: (value: string) =>
    invoke<GithubTokenStatus>("settings_github_token_set", { value }),
  settingsGithubTokenClear: () => invoke<void>("settings_github_token_clear"),

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

  // ---------- Keys (API keys + OAuth tokens) ----------
  /** List every stored ANTHROPIC_API_KEY — previews only, no secrets. */
  keyApiList: () => invoke<ApiKeySummary[]>("key_api_list"),
  /**
   * Add a new API key. Account is required — every key was created
   * under *some* account, and leaving that blank makes the row
   * un-findable by account later.
   */
  keyApiAdd: (label: string, token: string, accountUuid: string) =>
    invoke<ApiKeySummary>("key_api_add", { label, token, accountUuid }),
  keyApiRemove: (uuid: string) => invoke<void>("key_api_remove", { uuid }),
  /** Rename an API key. Label is user-owned metadata — no lookups
   *  key off it, so renames are display-only. */
  keyApiRename: (uuid: string, label: string) =>
    invoke<void>("key_api_rename", { uuid, label }),
  /** Pull the full plaintext secret out for clipboard. Sparingly. */
  keyApiCopy: (uuid: string) => invoke<string>("key_api_copy", { uuid }),
  /**
   * Validity ping against `GET /v1/models`. Resolves on a valid key;
   * rejects with a reason string ("rejected (invalid key)",
   * "rate-limited (retry in Ns)", …) that's safe to toast verbatim.
   * No DB write — result is transient.
   */
  keyApiProbe: (uuid: string) => invoke<void>("key_api_probe", { uuid }),

  /** List every stored CLAUDE_CODE_OAUTH_TOKEN — previews only. */
  keyOauthList: () => invoke<OauthTokenSummary[]>("key_oauth_list"),
  /**
   * Add a new OAuth token. Account tag is mandatory — the user picks
   * the account they ran `claude setup-token` against when created.
   */
  keyOauthAdd: (label: string, token: string, accountUuid: string) =>
    invoke<OauthTokenSummary>("key_oauth_add", {
      label,
      token,
      accountUuid,
    }),
  keyOauthRemove: (uuid: string) => invoke<void>("key_oauth_remove", { uuid }),
  /** Rename an OAuth token. See `keyApiRename`. */
  keyOauthRename: (uuid: string, label: string) =>
    invoke<void>("key_oauth_rename", { uuid, label }),
  keyOauthCopy: (uuid: string) => invoke<string>("key_oauth_copy", { uuid }),
  /**
   * Cached usage snapshot for the account the OAuth token belongs to.
   * Never hits Anthropic — peeks the in-memory cache populated by
   * `fetchAllUsage` / `refreshUsageFor` on the Accounts side. Returns
   * `null` if no cached snapshot exists yet for that account.
   */
  keyOauthUsageCached: (uuid: string) =>
    invoke<AccountUsage | null>("key_oauth_usage_cached", { uuid }),

  // ─── session_live (Activity feature) ─────────────────────────────

  /**
   * Start the live runtime (poll ~/.claude/sessions + tail transcripts).
   * Idempotent: repeated calls after a first successful start are
   * no-ops. The backend emits aggregate updates on the `live-all`
   * event channel and per-session deltas on `live::<sessionId>`.
   */
  /**
   * Partial update of the `activity_*` preference block. Any field
   * left undefined is preserved; the returned value is the refreshed
   * snapshot so the UI can round-trip without a separate GET.
   */
  preferencesSetActivity: (patch: {
    enabled?: boolean;
    consentSeen?: boolean;
    hideThinking?: boolean;
    excludedPaths?: string[];
  }) =>
    invoke<Preferences>("preferences_set_activity", {
      enabled: patch.enabled,
      consentSeen: patch.consentSeen,
      hideThinking: patch.hideThinking,
      excludedPaths: patch.excludedPaths,
    }),

  /** Partial update of the `notify_*` preference block. */
  preferencesSetNotifications: (patch: {
    onError?: boolean;
    onIdleDone?: boolean;
    onStuckMinutes?: number | null;
    onSpendUsd?: number | null;
  }) =>
    invoke<Preferences>("preferences_set_notifications", {
      onError: patch.onError,
      onIdleDone: patch.onIdleDone,
      onStuckMinutes: patch.onStuckMinutes,
      onSpendUsd: patch.onSpendUsd,
    }),

  sessionLiveStart: () => invoke<void>("session_live_start"),

  /** Stop the live runtime. Drops all detail subscribers. */
  sessionLiveStop: () => invoke<void>("session_live_stop"),

  /**
   * Synchronous snapshot of currently-live sessions. Used by
   * `useSessionLive` on first mount (before the first `live-all`
   * event arrives) and as the resync answer after a gap.
   */
  sessionLiveSnapshot: () =>
    invoke<LiveSessionSummary[]>("session_live_snapshot"),

  /**
   * One-session snapshot for resync after `resync_required`.
   * Returns `null` when the session is no longer live.
   */
  sessionLiveSessionSnapshot: (sessionId: string) =>
    invoke<LiveSessionSummary | null>("session_live_session_snapshot", {
      sessionId,
    }),

  /**
   * Subscribe to per-session detail deltas. Backend forwards every
   * delta as a `live::<sessionId>` Tauri event; the caller listens
   * via `useTauriEvent` or raw `listen`.
   *
   * Single-subscriber per session — concurrent calls for the same
   * id will reject. Callers must call `session_live_stop` then
   * `session_live_start` again to detach, or simply drop the local
   * listener (the backend detects the channel close on next send).
   */
  sessionLiveSubscribe: (sessionId: string) =>
    invoke<void>("session_live_subscribe", { sessionId }),

  /** Paired unsubscribe. Frontend listeners MUST call this before
   *  dropping their Tauri event listener — otherwise the backend
   *  task keeps forwarding until the session itself ends, and a
   *  re-subscribe on remount fails with AlreadySubscribed. */
  sessionLiveUnsubscribe: (sessionId: string) =>
    invoke<void>("session_live_unsubscribe", { sessionId }),

  /** Query the durable activity metrics store for the Trends view.
   *  Returns bucketed active-session counts + an error total for the
   *  requested window. Safe to call with `bucketCount: 0` → empty
   *  series. Unavailable metrics store → all-zero series, not
   *  an error. */
  activityTrends: (
    fromMs: number,
    toMs: number,
    bucketCount: number,
  ) =>
    invoke<ActivityTrends>("activity_trends", {
      fromMs,
      toMs,
      bucketCount,
    }),

  // Config section — P0 surface.
  configScan: (cwd?: string | null) =>
    invoke<ConfigTreeDto>("config_scan", { cwd: cwd ?? null }),
  configPreview: (nodeId: string) =>
    invoke<ConfigPreviewDto>("config_preview", { nodeId }),
  configSearchStart: (
    searchId: string,
    query: {
      text: string;
      regex?: boolean;
      case_sensitive?: boolean;
      scope_filter?: string[] | null;
    },
  ) => invoke<void>("config_search_start", { searchId, query }),
  configSearchCancel: (searchId: string) =>
    invoke<void>("config_search_cancel", { searchId }),
  configEffectiveSettings: (cwd?: string | null) =>
    invoke<ConfigEffectiveSettingsDto>("config_effective_settings", {
      cwd: cwd ?? null,
    }),
  configEffectiveMcp: (mode: McpSimulationMode, cwd?: string | null) =>
    invoke<ConfigEffectiveMcpDto>("config_effective_mcp", {
      cwd: cwd ?? null,
      mode,
    }),
  configListEditors: (force?: boolean) =>
    invoke<EditorCandidateDto[]>("config_list_editors", { force: !!force }),
  configGetEditorDefaults: () =>
    invoke<EditorDefaultsDto>("config_get_editor_defaults"),
  configSetEditorDefault: (kind: ConfigKind | null, editorId: string) =>
    invoke<void>("config_set_editor_default", { kind, editorId }),
  configOpenInEditorPath: (
    path: string,
    editorId: string | null,
    kindHint: ConfigKind | null,
  ) =>
    invoke<void>("config_open_in_editor_path", {
      path,
      editorId,
      kindHint,
    }),
  configWatchStart: (cwd?: string | null) =>
    invoke<void>("config_watch_start", { cwd: cwd ?? null }),
  configWatchStop: () => invoke<void>("config_watch_stop"),

  // Pricing — API-equivalent cost display for subscription users.
  pricingGet: () => invoke<PriceTableDto>("pricing_get"),
  pricingRefresh: () => invoke<PriceTableDto>("pricing_refresh"),
};
