import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import { api } from "../api";
import {
  useToasts,
  type DismissedToast,
  type Toast,
} from "../hooks/useToasts";
import { useRefresh } from "../hooks/useRefresh";
import { useDismissedIssues } from "../hooks/useDismissedIssues";
import { useBusy } from "../hooks/useBusy";
import { useActions } from "../hooks/useActions";
import { useOperations } from "../hooks/useOperations";
import { buildEmit, type EmitFn } from "../lib/notifications/dispatch";
import { hydrateCategoryPrefs } from "../lib/notifications/prefs";
import type { AccountSummary, AppStatus, CcIdentity } from "../types";

interface AppStateValue {
  // Toasts (lifted so shell can render them above the accounts view too).
  toasts: Toast[];
  pushToast: ReturnType<typeof useToasts>["pushToast"];
  dismissToast: (id: number) => void;
  /**
   * Unified notification dispatcher. Call sites pass a typed event;
   * the facade picks surfaces from routing, writes one log entry,
   * and fans out to primitives with the in-place migration shim
   * suppressing double-logs. See
   * `src/lib/notifications/dispatch.ts` and
   * `dev-docs/notification-system-plan.md` (Phase 1).
   *
   * Direct `pushToast` calls still work during the migration window
   * (Phase 1 → Phase 3). New code should prefer `emit({...})`.
   */
  emit: EmitFn;
  /**
   * Last toast that fully dismissed. The status bar reads this and
   * echoes the message for a few seconds so the user can re-read what
   * just scrolled by — `null` once the echo has been cleared.
   */
  lastDismissed: DismissedToast | null;
  /** Clear the echo segment when its fade window elapses. */
  clearLastDismissed: () => void;

  // Refresh state — single source shared between AppShell banner and
  // AccountsSection so there's only one `/profile` + `/verify_all`
  // round per tick.
  status: AppStatus | null;
  accounts: AccountSummary[];
  loadError: string | null;
  keychainIssue: string | null;
  syncError: string | null;
  /**
   * Epoch-ms timestamp of the last `sync_from_current_cc` result that
   * came back with `auth rejected:` — CC's refresh_token is dead, the
   * user must sign in again. Drives the "Sign in again" banner and a
   * 60 s cooldown in `useRefresh` to stop focus-thrashing the endpoint.
   * Null when the latest sync either succeeded or failed transiently.
   */
  authRejectedAt: number | null;
  ccIdentity: CcIdentity | null;
  verifying: boolean;
  refresh: () => Promise<void>;

  // Per-issue 24 h snooze store — banner + in-card alerts both gate on it.
  isDismissed: (id: string) => boolean;
  dismiss: (id: string) => void;
  clearDismissed: (id: string) => void;
  /** Live (non-expired) dismissed-issue keys; for the App.tsx snooze
   *  auto-clear effect to reconcile entries from a prior renderer
   *  lifetime against the currently-live issue set. */
  knownDismissedKeys: () => string[];

  // Action helpers — single instance so sidebar binds and
  // AccountsSection share the busy keyring, toast queue, and
  // preflight gate.
  busyKeys: Set<string>;
  actions: ReturnType<typeof useActions>;
  /**
   * Unified CLI swap entry point. Runs the split-brain preflight; if
   * a live CC process is detected, parks the account in
   * `splitBrainPending` for the shell-level ConfirmDialog. Otherwise
   * delegates to `actions.useCli`.
   */
  requestCliSwap: (a: AccountSummary) => Promise<void>;
  splitBrainPending: AccountSummary | null;
  dismissSplitBrain: () => void;
  confirmSplitBrain: () => void;

  /**
   * Pending destructive Desktop confirmation. The shell renders
   * `<DesktopConfirmDialog>` when this is non-null. The three
   * variants mirror the three destructive-confirm surfaces Codex
   * flagged in the follow-up review: sign-out, bind-overwrite, and
   * (future-reserved) swap-with-live-session.
   */
  desktopConfirmPending: DesktopConfirmRequest | null;
  requestDesktopSignOut: () => void;
  requestDesktopOverwrite: (a: AccountSummary) => void;
  dismissDesktopConfirm: () => void;
  confirmDesktopPending: () => void;

  /**
   * Pending account-removal confirmation. Shell-level so the command
   * palette (mounted in AppShell) and the Accounts context menu route
   * through the same ConfirmDialog. Actual removal still flows through
   * `actions.performRemove`, which carries the undo toast.
   */
  removeConfirmPending: AccountSummary | null;
  requestRemoveAccount: (a: AccountSummary) => void;
  dismissRemoveConfirm: () => void;
  confirmRemoveAccount: () => void;
}

export type DesktopConfirmRequest =
  | { kind: "sign_out" }
  | { kind: "overwrite_profile"; account: AccountSummary };

const AppStateContext = createContext<AppStateValue | null>(null);

/**
 * App-wide mount point for refresh state, toasts, and dismissed-issue
 * tracking. Two independent `useRefresh` calls used to run in parallel
 * (App.tsx via `useAccounts`, AccountsSection via its own call) —
 * doubling `/profile` and `verify_all_accounts` traffic, and letting
 * drift banners die in the gap between them. Centralising here lets
 * `useStatusIssues` fire once at shell level off the same state.
 */
export function AppStateProvider({ children }: { children: ReactNode }) {
  const {
    toasts,
    pushToast: pushToastPrimitive,
    dismissToast,
    lastDismissed,
    clearLastDismissed,
  } = useToasts();
  const {
    status,
    accounts,
    loadError,
    keychainIssue,
    syncError,
    authRejectedAt,
    ccIdentity,
    verifying,
    refresh,
  } = useRefresh(pushToastPrimitive);
  const { isDismissed, dismiss, clear, knownKeys } = useDismissedIssues();
  const busy = useBusy();
  const { open: openOpModal } = useOperations();

  // Build the unified emit dispatcher. Re-built only when the toast
  // primitive identity changes (it's stable across renders, so this
  // memo lands once per AppStateProvider lifetime in practice).
  const emit = useMemo<EmitFn>(
    () => buildEmit({ pushToast: pushToastPrimitive }),
    [pushToastPrimitive],
  );

  // Public `pushToast` — wraps emit() with `category: configEdited`
  // so every legacy call site automatically routes through the
  // unified pipeline. Sites that want a specific category should
  // call `emit({...})` directly; everything else gets a sane
  // P2-acknowledge default that the Settings → Notifications pane
  // can mute via the `configEdited` toggle.
  //
  // Same call signature as `useToasts.pushToast` so consumers don't
  // need to change. Returns void (matches the primitive); errors
  // inside emit() are swallowed there.
  const pushToast = useCallback<typeof pushToastPrimitive>(
    (kind, text, onUndo, opts) => {
      void emit({
        category: "configEdited",
        kind: kind === "error" ? "error" : "info",
        title: text,
        dedupeKey: opts?.dedupeKey,
        toastAction: onUndo
          ? {
              label: opts?.undoLabel ?? "Undo",
              onPress: onUndo,
              onCommit: opts?.onCommit,
              timeoutMs: opts?.undoMs,
            }
          : undefined,
        // Audit-fix Medium #9: forward durationMs through to the
        // primitive — callers that pass it (e.g. sticky error
        // toasts the user must dismiss manually) silently regressed
        // to the default before this fix.
        toastDurationMs: opts?.durationMs,
      });
    },
    [emit],
  );

  const actions = useActions({
    pushToast,
    refresh,
    withBusy: busy.withBusy,
    openOpModal,
  });

  // Hydrate the CategoryPrefs cache once on mount so emit() reads
  // real user preferences. Re-hydrate when ANY preference setter
  // fires (audit-fix #3): the legacy scalar setters
  // (`preferences_set_notifications`,
  // `preferences_set_service_status`, `preferences_set_activity`)
  // now mirror into `category_prefs` server-side, but the renderer
  // cache only knew about updates made through
  // `preferences_category_pref_set`. Without this listener, toggling
  // via Settings → Notifications' legacy scalar rows would leave
  // emit() reading stale category state.
  //
  // Fire-and-forget — emit() falls back to sensible defaults until
  // the cache lands. Module-level dynamic import on @tauri-apps so
  // non-Tauri test environments don't blow up.
  useEffect(() => {
    void hydrateCategoryPrefs();
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    void import("@tauri-apps/api/event")
      .then(({ listen }) =>
        listen("cp-prefs-changed", () => {
          void hydrateCategoryPrefs();
        }),
      )
      .then((fn) => {
        if (cancelled) {
          if (typeof fn === "function") fn();
        } else {
          unlisten = fn;
        }
      })
      .catch(() => {
        /* non-tauri env */
      });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  const [splitBrainPending, setSplitBrainPending] =
    useState<AccountSummary | null>(null);

  const requestCliSwap = useCallback(
    async (a: AccountSummary) => {
      try {
        if (await api.cliIsCcRunning()) {
          setSplitBrainPending(a);
          return;
        }
      } catch {
        // Preflight failure falls through; the server-side gate in
        // swap.rs still rejects live conflicts with a clear error.
      }
      await actions.useCli(a);
    },
    [actions],
  );

  const dismissSplitBrain = useCallback(() => setSplitBrainPending(null), []);
  const confirmSplitBrain = useCallback(() => {
    const target = splitBrainPending;
    setSplitBrainPending(null);
    if (target) void actions.useCli(target, true);
  }, [actions, splitBrainPending]);

  const [desktopConfirmPending, setDesktopConfirmPending] =
    useState<DesktopConfirmRequest | null>(null);
  const requestDesktopSignOut = useCallback(
    () => setDesktopConfirmPending({ kind: "sign_out" }),
    [],
  );
  const requestDesktopOverwrite = useCallback(
    (account: AccountSummary) =>
      setDesktopConfirmPending({ kind: "overwrite_profile", account }),
    [],
  );
  const dismissDesktopConfirm = useCallback(
    () => setDesktopConfirmPending(null),
    [],
  );
  const confirmDesktopPending = useCallback(() => {
    const req = desktopConfirmPending;
    setDesktopConfirmPending(null);
    if (!req) return;
    switch (req.kind) {
      case "sign_out":
        void actions.clearDesktopConfirmed(true);
        break;
      case "overwrite_profile":
        void actions.adoptDesktopForce(req.account, true);
        break;
    }
  }, [actions, desktopConfirmPending]);

  const [removeConfirmPending, setRemoveConfirmPending] =
    useState<AccountSummary | null>(null);
  const requestRemoveAccount = useCallback(
    (a: AccountSummary) => setRemoveConfirmPending(a),
    [],
  );
  const dismissRemoveConfirm = useCallback(
    () => setRemoveConfirmPending(null),
    [],
  );
  const confirmRemoveAccount = useCallback(() => {
    const target = removeConfirmPending;
    setRemoveConfirmPending(null);
    if (target) actions.performRemove(target);
  }, [actions, removeConfirmPending]);

  const value = useMemo<AppStateValue>(
    () => ({
      toasts,
      pushToast,
      emit,
      dismissToast,
      lastDismissed,
      clearLastDismissed,
      status,
      accounts,
      loadError,
      keychainIssue,
      syncError,
      authRejectedAt,
      ccIdentity,
      verifying,
      refresh,
      isDismissed,
      dismiss,
      clearDismissed: clear,
      knownDismissedKeys: knownKeys,
      busyKeys: busy.busyKeys,
      actions,
      requestCliSwap,
      splitBrainPending,
      dismissSplitBrain,
      confirmSplitBrain,
      desktopConfirmPending,
      requestDesktopSignOut,
      requestDesktopOverwrite,
      dismissDesktopConfirm,
      confirmDesktopPending,
      removeConfirmPending,
      requestRemoveAccount,
      dismissRemoveConfirm,
      confirmRemoveAccount,
    }),
    [
      toasts,
      pushToast,
      emit,
      dismissToast,
      lastDismissed,
      clearLastDismissed,
      status,
      accounts,
      loadError,
      keychainIssue,
      syncError,
      authRejectedAt,
      ccIdentity,
      verifying,
      refresh,
      isDismissed,
      dismiss,
      clear,
      knownKeys,
      busy.busyKeys,
      actions,
      requestCliSwap,
      splitBrainPending,
      dismissSplitBrain,
      confirmSplitBrain,
      desktopConfirmPending,
      requestDesktopSignOut,
      requestDesktopOverwrite,
      dismissDesktopConfirm,
      confirmDesktopPending,
      removeConfirmPending,
      requestRemoveAccount,
      dismissRemoveConfirm,
      confirmRemoveAccount,
    ],
  );

  return (
    <AppStateContext.Provider value={value}>
      {children}
    </AppStateContext.Provider>
  );
}

/**
 * Convenience hook returning just the `emit()` dispatcher. New
 * notification call sites should prefer this over reading
 * `pushToast` directly so the migration to per-category prefs and
 * unified logging (Phase 1.5+) only touches the dispatcher.
 */
export function useEmit(): EmitFn {
  return useAppState().emit;
}

export function useAppState(): AppStateValue {
  const v = useContext(AppStateContext);
  if (!v) {
    throw new Error(
      "useAppState must be called inside <AppStateProvider>. Check App.tsx.",
    );
  }
  return v;
}
