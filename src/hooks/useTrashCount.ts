import { useCallback, useEffect, useState } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { api } from "../api";

/**
 * Polls `sessionTrashList` and returns the number of entries in the
 * trash. Used by the Sessions section to draw a dot on the Cleanup
 * tab when trash is non-empty. Refreshes on window focus and when
 * the `tray-usage-refreshed` event lands (cheap proxy for "state
 * changed elsewhere"). Swallows errors quietly — the count is a
 * decoration, not a functional dependency.
 */
export function useTrashCount(): { count: number; refresh: () => void } {
  const [count, setCount] = useState(0);

  const refresh = useCallback(async () => {
    try {
      const listing = await api.sessionTrashList();
      setCount(listing.entries.length);
    } catch {
      // Keep the last known count. If sessionTrashList throws, the
      // cleanup tab still works — the dot just goes stale.
    }
  }, []);

  useEffect(() => {
    void refresh();
    const onFocus = () => void refresh();
    window.addEventListener("focus", onFocus);

    // Trash mutations don't emit a bespoke event yet; listen for the
    // existing tray-usage-refreshed as a best-effort nudge, and the
    // section's own onChange hooks call refresh() directly. If we
    // grow a bespoke event we'll add it here.
    let unlisten: UnlistenFn | undefined;
    listen("tray-usage-refreshed", () => void refresh())
      .then((fn) => {
        unlisten = fn;
      })
      .catch(() => {});

    return () => {
      window.removeEventListener("focus", onFocus);
      unlisten?.();
    };
  }, [refresh]);

  return { count, refresh };
}
