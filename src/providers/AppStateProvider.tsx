import {
  createContext,
  useCallback,
  useContext,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import { api } from "../api";
import { useToasts, type Toast } from "../hooks/useToasts";
import { useRefresh } from "../hooks/useRefresh";
import { useDismissedIssues } from "../hooks/useDismissedIssues";
import { useBusy } from "../hooks/useBusy";
import { useActions } from "../hooks/useActions";
import type { AccountSummary, AppStatus, CcIdentity } from "../types";

interface AppStateValue {
  // Toasts (lifted so shell can render them above the accounts view too).
  toasts: Toast[];
  pushToast: ReturnType<typeof useToasts>["pushToast"];
  dismissToast: (id: number) => void;

  // Refresh state — single source shared between AppShell banner and
  // AccountsSection so there's only one `/profile` + `/verify_all`
  // round per tick.
  status: AppStatus | null;
  accounts: AccountSummary[];
  loadError: string | null;
  keychainIssue: string | null;
  syncError: string | null;
  ccIdentity: CcIdentity | null;
  verifying: boolean;
  refresh: () => Promise<void>;

  // Per-issue 24 h snooze store — banner + in-card alerts both gate on it.
  isDismissed: (id: string) => boolean;
  dismiss: (id: string) => void;
  clearDismissed: (id: string) => void;

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
}

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
  const { toasts, pushToast, dismissToast } = useToasts();
  const {
    status,
    accounts,
    loadError,
    keychainIssue,
    syncError,
    ccIdentity,
    verifying,
    refresh,
  } = useRefresh(pushToast);
  const { isDismissed, dismiss, clear } = useDismissedIssues();
  const busy = useBusy();
  const actions = useActions({
    pushToast,
    refresh,
    withBusy: busy.withBusy,
    addBusy: busy.addBusy,
    removeBusy: busy.removeBusy,
  });

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

  const value = useMemo<AppStateValue>(
    () => ({
      toasts,
      pushToast,
      dismissToast,
      status,
      accounts,
      loadError,
      keychainIssue,
      syncError,
      ccIdentity,
      verifying,
      refresh,
      isDismissed,
      dismiss,
      clearDismissed: clear,
      busyKeys: busy.busyKeys,
      actions,
      requestCliSwap,
      splitBrainPending,
      dismissSplitBrain,
      confirmSplitBrain,
    }),
    [
      toasts,
      pushToast,
      dismissToast,
      status,
      accounts,
      loadError,
      keychainIssue,
      syncError,
      ccIdentity,
      verifying,
      refresh,
      isDismissed,
      dismiss,
      clear,
      busy.busyKeys,
      actions,
      requestCliSwap,
      splitBrainPending,
      dismissSplitBrain,
      confirmSplitBrain,
    ],
  );

  return (
    <AppStateContext.Provider value={value}>
      {children}
    </AppStateContext.Provider>
  );
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
