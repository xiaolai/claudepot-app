// Account, identity, desktop slot, login/register, usage, verify.
// Sharded from src/api.ts; src/api/index.ts merges every
// domain slice into the canonical `api` object.

import { invoke } from "@tauri-apps/api/core";
import { invokeWithTimeout } from "./invokeWithTimeout";
import type {
  AccountSummary,
  AccountSummaryBasic,
  AppStatus,
  CcIdentity,
  DesktopAdoptOutcome,
  DesktopClearOutcome,
  DesktopIdentity,
  DesktopSyncOutcome,
  RegisterOutcome,
  RemoveOutcome,
  RunningOpInfo,
  UsageEntry,
  UsageMap,
} from "../types";

export const accountApi = {
  appStatus: () => invoke<AppStatus>("app_status"),
  /**
   * Confirmed quit. Called from QuitConfirm after the user agrees to
   * abandon in-flight ops. The Rust side calls `app.exit(0)` directly;
   * there is no second gate.
   */
  quitNow: () => invoke<void>("quit_now"),
  /**
   * Whether the in-app updater is supported on this install. False on
   * Linux when the binary isn't running from an AppImage (e.g. a .deb
   * install or a system package), where in-place replacement would
   * race with apt. The frontend hides the auto-update UI when this
   * returns false.
   */
  updaterSupported: () => invoke<boolean>("updater_supported"),

  /**
   * Push the current count of "alerting" sessions (errored or stuck)
   * to the tray. The macOS menubar shows it next to the icon; Windows
   * and Linux receive it in the tooltip. Cheap — fires whenever the
   * count changes; the backend takes the fast path that updates
   * title + tooltip without rebuilding the full tray menu.
   */
  traySetAlertCount: (count: number) =>
    invoke<void>("tray_set_alert_count", { count }),

  /// Idempotent startup adoption: if CC holds credentials for one of the
  /// registered accounts, imports them into the matching slot. Returns
  /// the synced email (empty string when nothing matched).
  ///
  /// Bounded at 15s: one keychain read + one save + one /profile HTTP
  /// call. See `./invokeWithTimeout.ts` for why JS-side ceilings are
  /// load-bearing alongside the 5s-per-subprocess Rust budget.
  syncFromCurrentCc: () =>
    invokeWithTimeout<string>("sync_from_current_cc", undefined, 15_000),
  /// macOS-only: request a native keychain-unlock dialog. The user's
  /// password is entered directly into macOS's own trusted prompt and
  /// never reaches Claudepot.
  unlockKeychain: () => invoke<void>("unlock_keychain"),
  /// Reveal a path in the native file manager (Finder / Explorer /
  /// file manager). Walks up to the nearest existing parent if the
  /// exact path is gone (orphan projects still "open parent").
  revealInFinder: (path: string) => invoke<void>("reveal_in_finder", { path }),
  /// Bounded at 30s: N × `token_health` → `swap::load_private` calls,
  /// each capped at 5s on the Rust side. 30s carries up to ~6
  /// keychain-stalled accounts before the JS ceiling fires. See
  /// `./invokeWithTimeout.ts`.
  accountList: () =>
    invokeWithTimeout<AccountSummary[]>("account_list", undefined, 30_000),
  /** Keychain-free lean list — use when you only need identity
   *  fields (uuid / email / org / subscription / active flags).
   *  Much faster than `accountList` because it skips the per-account
   *  `token_health` call that hits macOS Keychain. */
  accountListBasic: () =>
    invoke<AccountSummaryBasic[]>("account_list_basic"),
  /// Bounded at 30s: full swap path does multiple keychain reads
  /// (`storage::load`, `storage::save`) plus an HTTP `/profile` fetch
  /// for identity verification.
  cliUse: (email: string, force = false) =>
    invokeWithTimeout<void>("cli_use", { email, force }, 30_000),
  /// Cheap preflight used before cli_use to decide whether to raise
  /// the split-brain confirmation dialog.
  cliIsCcRunning: () => invoke<boolean>("cli_is_cc_running"),
  /// Bounded at 30s: Desktop slot swap touches the CC-credentials
  /// keychain via `cli_backend::keychain` (5s Rust ceiling per
  /// subprocess) plus profile-storage moves on disk. Same policy
  /// symmetry as `cliUse` — see `./invokeWithTimeout.ts`.
  desktopUse: (email: string, noLaunch: boolean) =>
    invokeWithTimeout<void>("desktop_use", { email, noLaunch }, 30_000),
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
  /// Adopt the live Desktop session into `uuid`'s snapshot directory.
  /// Always verifies identity via the authoritative Decrypted path
  /// before mutating — fast-path candidates cannot drive adoption.
  ///
  /// Bounded at 30s: Decrypted identity probe (keychain read +
  /// SecCryptoKey unwrap) plus profile snapshot mkdir/copy.
  desktopAdopt: (uuid: string, overwrite: boolean) =>
    invokeWithTimeout<DesktopAdoptOutcome>(
      "desktop_adopt",
      { uuid, overwrite },
      30_000,
    ),
  /// Sign Desktop out. `keepSnapshot=true` (default) preserves the
  /// current session as a snapshot under the active account.
  ///
  /// Bounded at 15s: identity probe + snapshot move.
  desktopClear: (keepSnapshot: boolean) =>
    invokeWithTimeout<DesktopClearOutcome>(
      "desktop_clear",
      { keepSnapshot },
      15_000,
    ),
  /// Startup / window-focus sync. Read-only (no disk mutation); at
  /// most refreshes the active_desktop pointer cache.
  ///
  /// Bounded at 15s: identity probe (CC-credentials keychain read).
  syncFromCurrentDesktop: () =>
    invokeWithTimeout<DesktopSyncOutcome>(
      "sync_from_current_desktop",
      undefined,
      15_000,
    ),
  desktopLaunch: () => invoke<void>("desktop_launch"),
  /// Bounded at 15s: one `storage::save_private` + one `/profile`
  /// HTTP fetch.
  accountAddFromCurrent: () =>
    invokeWithTimeout<RegisterOutcome>(
      "account_add_from_current",
      undefined,
      15_000,
    ),
  /// Browser OAuth onboarding — spawns `claude auth login` in a temp
  /// config dir, returns when the user finishes (or errors). The
  /// refresh token never crosses the IPC bridge; everything is handled
  /// by claudepot-core on the Rust side.
  accountRegisterFromBrowser: () =>
    invoke<RegisterOutcome>("account_register_from_browser"),
  /// Async variant of `accountRegisterFromBrowser`: returns the op_id
  /// immediately. Subscribe to `op-progress::<op_id>` for `LoginPhase`
  /// events and call `accountLoginStatus` once the terminal event lands
  /// (the same status endpoint serves both register + login flows since
  /// they share the `RunningOpInfo` shape).
  accountRegisterFromBrowserStart: () =>
    invoke<string>("account_register_from_browser_start"),
  // Token-based onboarding is CLI-only — the refresh token must never enter
  // the webview JS heap. Browser onboarding above is the GUI equivalent.
  /// Re-log in via browser (opens Claude's OAuth flow) and imports the
  /// resulting blob into the given account's slot. Can take several
  /// minutes while the user completes auth in the browser.
  accountLogin: (uuid: string) => invoke<void>("account_login", { uuid }),
  /// Async variant of `accountLogin`: returns the op_id immediately so
  /// the IPC worker isn't held for the full subprocess + OAuth wait.
  accountLoginStart: (uuid: string) =>
    invoke<string>("account_login_start", { uuid }),
  /// Poll the current state of an in-flight login op. Used as a backstop
  /// in case an `op-progress` event drops; the modal reads the final
  /// `RunningOpInfo` here once the terminal event fires.
  accountLoginStatus: (opId: string) =>
    invoke<RunningOpInfo | null>("account_login_status", { opId }),
  accountLoginCancel: () => invoke<void>("account_login_cancel"),
  /// Bounded at 15s: one `storage::delete` (keychain delete + file
  /// delete) plus a DB write.
  accountRemove: (uuid: string) =>
    invokeWithTimeout<RemoveOutcome>("account_remove", { uuid }, 15_000),
  /// Bounded at 60s: N × `load_access_token` (each a keychain read,
  /// 5s ceiling) plus N × usage HTTP calls (each ~5-10s). For up to
  /// ~6 accounts the worst case lands well under 60s.
  fetchAllUsage: () =>
    invokeWithTimeout<UsageMap>("fetch_all_usage", undefined, 60_000),
  /// Invalidate cache + cooldown for a single account then refetch.
  /// Scoped alternative to fetchAllUsage for per-row Retry buttons.
  /// Bounded at 15s: one keychain read + one usage HTTP call.
  refreshUsageFor: (uuid: string) =>
    invokeWithTimeout<UsageEntry>("refresh_usage_for", { uuid }, 15_000),
  /// Reconcile every account's blob identity against `/api/oauth/profile`.
  /// Returns the refreshed list so the caller can re-render without a
  /// separate `accountList` round-trip. Slow — one HTTP call per account
  /// with credentials.
  ///
  /// Bounded at 60s: N × `(load_private + /profile)`. For locked-
  /// keychain stalls the JS ceiling fires before the user assumes the
  /// whole pane is dead.
  verifyAllAccounts: () =>
    invokeWithTimeout<AccountSummary[]>(
      "verify_all_accounts",
      undefined,
      60_000,
    ),
  /// Async variant of `verifyAllAccounts`: returns the op_id immediately
  /// so per-account events can drive inline row badge updates instead of
  /// blocking the IPC worker on N round-trips. Subscribe to
  /// `op-progress::<op_id>` for both `OperationProgressEvent` (phase
  /// advance + terminal) and `VerifyAccountEvent` (per-row payloads).
  verifyAllAccountsStart: () => invoke<string>("verify_all_accounts_start"),
  /// Poll the current state of an in-flight verify_all op.
  verifyAllAccountsStatus: (opId: string) =>
    invoke<RunningOpInfo | null>("verify_all_accounts_status", { opId }),
  /// Verify a single account — fast, single /profile round-trip. Used
  /// by the per-row context menu and command palette.
  ///
  /// Bounded at 15s: one keychain read + one HTTP call.
  verifyAccount: (uuid: string) =>
    invokeWithTimeout<AccountSummary>("verify_account", { uuid }, 15_000),
  /// Ground-truth "what is CC currently authenticated as". Reads the
  /// shared slot + calls /profile. Never throws — errors land in the
  /// returned `error` field so the UI can render them as a banner.
  currentCcIdentity: () => invoke<CcIdentity>("current_cc_identity"),

};
