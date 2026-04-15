import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "../api";
import type { AccountSummary, AppStatus } from "../types";

export function useRefresh(pushToast: (kind: "info" | "error", text: string) => void) {
  const [status, setStatus] = useState<AppStatus | null>(null);
  const [accounts, setAccounts] = useState<AccountSummary[]>([]);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [keychainIssue, setKeychainIssue] = useState<string | null>(null);
  const lastRefreshRef = useRef(0);
  const refreshingRef = useRef(false);

  const refresh = useCallback(async () => {
    if (refreshingRef.current) return;
    refreshingRef.current = true;
    lastRefreshRef.current = Date.now();
    try {
      try {
        await api.syncFromCurrentCc();
        setKeychainIssue(null);
      } catch (e) {
        const msg = `${e}`;
        if (msg.toLowerCase().includes("keychain is locked")) {
          setKeychainIssue(msg);
        } else {
          setKeychainIssue(null);
          // eslint-disable-next-line no-console
          console.warn("sync_from_current_cc failed:", msg);
        }
      }
      // First fetch: cheap, renders quickly from DB state.
      const [s, list] = await Promise.all([
        api.appStatus(),
        api.accountList(),
      ]);
      setStatus(s);
      setAccounts(list);
      setLoadError(null);

      // Background reconciliation: verify every account's blob identity
      // against /api/oauth/profile. Slow (one HTTP per account), so we
      // don't block the initial render on it. Drift / rejected / network
      // outcomes propagate into the next accounts state update.
      api
        .verifyAllAccounts()
        .then((verified) => setAccounts(verified))
        .catch((e) => {
          // eslint-disable-next-line no-console
          console.warn("verify_all_accounts failed:", e);
        });
    } catch (e) {
      const msg = `${e}`;
      setLoadError(msg);
      pushToast("error", `refresh failed: ${msg}`);
    } finally {
      refreshingRef.current = false;
    }
  }, [pushToast]);

  useEffect(() => {
    refresh();
    const onFocus = () => {
      if (Date.now() - lastRefreshRef.current > 2000) {
        refresh();
      }
    };
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, [refresh]);

  return { status, accounts, loadError, keychainIssue, refresh };
}
