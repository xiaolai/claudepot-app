import {
  createContext,
  useContext,
  useMemo,
  type ReactNode,
} from "react";
import { useToasts, type Toast } from "../hooks/useToasts";
import { useRefresh } from "../hooks/useRefresh";
import { useDismissedIssues } from "../hooks/useDismissedIssues";
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
