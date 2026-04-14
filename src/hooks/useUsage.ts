import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "../api";
import type { UsageMap } from "../types";

/** Fetch usage for all accounts. Refreshes on window focus (debounced 5s)
 *  and on manual `refreshUsage()` calls. Never throws — errors are
 *  silently swallowed (the backend already absorbs rate-limit states). */
export function useUsage() {
  const [usage, setUsage] = useState<UsageMap>({});
  const lastRef = useRef(0);
  const fetchingRef = useRef(false);

  const refreshUsage = useCallback(async () => {
    if (fetchingRef.current) return;
    fetchingRef.current = true;
    lastRef.current = Date.now();
    try {
      const data = await api.fetchAllUsage();
      setUsage(data);
    } catch {
      // Silently ignore — stale data stays in state.
    } finally {
      fetchingRef.current = false;
    }
  }, []);

  useEffect(() => {
    refreshUsage();
    const onFocus = () => {
      if (Date.now() - lastRef.current > 5000) {
        refreshUsage();
      }
    };
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, [refreshUsage]);

  return { usage, refreshUsage };
}
