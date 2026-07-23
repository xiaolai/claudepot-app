import { emit } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "../api";
import { USAGE_REFETCH_EVENT } from "../lib/events";
import { useTauriEvent } from "./useTauriEvent";
import type { UsageMap } from "../types";

/** Fetch usage for all accounts. Refreshes on window focus (debounced 5s)
 *  and on manual `refreshUsage()` calls. Never throws — errors are
 *  silently swallowed (the backend already absorbs rate-limit states). */
export function useUsage() {
  const [usage, setUsage] = useState<UsageMap>({});
  const [lastFetchedAt, setLastFetchedAt] = useState<number | null>(null);
  const lastRef = useRef(0);
  const fetchingRef = useRef(false);

  const refreshUsage = useCallback(async () => {
    if (fetchingRef.current) return;
    fetchingRef.current = true;
    lastRef.current = Date.now();
    try {
      const data = await api.fetchAllUsage();
      setUsage(data);
      setLastFetchedAt(Date.now());
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
      setLastFetchedAt(Date.now());
      emit("rebuild-tray-menu").catch(() => {});
    } catch {
      // Silently ignore — stale entry stays in state.
    }
  }, []);

  // When the user clicks "Refresh" in the tray's Usage submenu,
  // the backend re-fetches and emits tray-usage-refreshed. Pull the
  // fresh cache into the webview so the card values match the tray.
  // (useTauriEvent owns the audit-T4-6 late-resolve race internally.)
  useTauriEvent("tray-usage-refreshed", () => {
    void refreshUsage();
  });

  // A background token heal (or a UI-driven verify) just rotated an
  // account's credentials — re-pull so its card flips from "token
  // expired" to live numbers on its own, instead of waiting for the
  // user to reach for Refresh. Emitters: the backend
  // token_refresh_orchestrator, and the frontend verify paths
  // (runVerifyAll / runVerifyAccount).
  // Channel mirrors src-tauri/src/events.rs::USAGE_REFETCH.
  useTauriEvent(USAGE_REFETCH_EVENT, () => {
    void refreshUsage();
  });

  useEffect(() => {
    refreshUsage();
    const onFocus = () => {
      if (Date.now() - lastRef.current > 5000) {
        refreshUsage();
      }
    };
    window.addEventListener("focus", onFocus);
    return () => {
      window.removeEventListener("focus", onFocus);
    };
  }, [refreshUsage]);

  return { usage, refreshUsage, refreshUsageFor, lastFetchedAt };
}
