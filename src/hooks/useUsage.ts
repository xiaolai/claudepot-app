import { emit, listen, type UnlistenFn } from "@tauri-apps/api/event";
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
      // Keep the tray's Usage submenu in sync with what the cards show.
      // Best-effort: swallow failures so a missing window/tray doesn't
      // break the happy path.
      emit("rebuild-tray-menu").catch(() => {});
    } catch {
      // Silently ignore — stale data stays in state.
    } finally {
      fetchingRef.current = false;
    }
  }, []);

  /**
   * Per-account refresh. Targets ONE uuid — doesn't retrigger fetches
   * for the rest of the accounts. Used by row-level Retry buttons so
   * clicking "Retry" on a rate-limited account doesn't spam healthy
   * accounts with needless HTTP calls.
   */
  const refreshUsageFor = useCallback(async (uuid: string) => {
    try {
      const entry = await api.refreshUsageFor(uuid);
      setUsage((prev) => ({ ...prev, [uuid]: entry }));
      emit("rebuild-tray-menu").catch(() => {});
    } catch {
      // Silently ignore — stale entry stays in state.
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

    // When the user clicks "Refresh" in the tray's Usage submenu,
    // the backend re-fetches and emits tray-usage-refreshed. Pull the
    // fresh cache into the webview so the card values match the tray.
    let unlisten: UnlistenFn | undefined;
    listen("tray-usage-refreshed", () => {
      refreshUsage();
    })
      .then((fn) => {
        unlisten = fn;
      })
      .catch(() => {});

    return () => {
      window.removeEventListener("focus", onFocus);
      unlisten?.();
    };
  }, [refreshUsage]);

  return { usage, refreshUsage, refreshUsageFor };
}
