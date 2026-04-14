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
      const [s, list] = await Promise.all([
        api.appStatus(),
        api.accountList(),
      ]);
      setStatus(s);
      setAccounts(list);
      setLoadError(null);
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
