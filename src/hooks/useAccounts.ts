import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "../api";
import type { AccountSummary } from "../types";

/**
 * Global account list + loading state. Previously lived inline in
 * AccountsSection; promoted here so the shell (Sidebar swap targets
 * + StatusBar stats + Command palette) can render accounts without
 * waiting for the Accounts screen to mount.
 *
 * `refresh()` pulls the list again; also invoked on window focus
 * debounced at 2s so a background CLI switch reflects immediately
 * when the user comes back.
 */
export function useAccounts(): {
  accounts: AccountSummary[];
  loading: boolean;
  error: string | null;
  refresh: () => Promise<void>;
} {
  const [accounts, setAccounts] = useState<AccountSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const focusTimer = useRef<number | null>(null);

  const refresh = useCallback(async () => {
    try {
      setError(null);
      const list = await api.accountList();
      setAccounts(list);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    let cancelled = false;
    // Startup sync: best-effort. If CC holds credentials matching a
    // registered account, the backend imports them into the right slot
    // before we load the list.
    (async () => {
      try {
        await api.syncFromCurrentCc();
      } catch {
        // non-fatal — we still want to render whatever the store has
      }
      if (!cancelled) await refresh();
    })();
    return () => {
      cancelled = true;
    };
  }, [refresh]);

  useEffect(() => {
    const onFocus = () => {
      if (focusTimer.current) window.clearTimeout(focusTimer.current);
      focusTimer.current = window.setTimeout(() => {
        refresh();
      }, 2000);
    };
    window.addEventListener("focus", onFocus);
    return () => {
      window.removeEventListener("focus", onFocus);
      if (focusTimer.current) window.clearTimeout(focusTimer.current);
    };
  }, [refresh]);

  return { accounts, loading, error, refresh };
}

export interface TargetBinding {
  cli: string | null;
  desktop: string | null;
}

/** Derive `{cli, desktop}` UUID binding from the active flags. */
export function bindingFrom(accounts: AccountSummary[]): TargetBinding {
  return {
    cli: accounts.find((a) => a.is_cli_active)?.uuid ?? null,
    desktop: accounts.find((a) => a.is_desktop_active)?.uuid ?? null,
  };
}
