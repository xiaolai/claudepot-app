import { useCallback, useEffect, useRef, useState } from "react";
import { api } from "../api";
import { useTauriEvent } from "./useTauriEvent";
import type { Preferences } from "../types";

/**
 * The status bar's window pin — `window_always_on_top` in
 * `preferences.json`, which keeps the main window in front of every
 * other app's windows.
 *
 * Reads the preference on mount and follows `cp-prefs-changed`, so a
 * change made from anywhere else moves the button too. `toggle` is
 * optimistic and reverts on failure: the backend changes the window
 * level before it persists and undoes it if the save fails, so a
 * rejected call means the window is where it was, and the button must
 * say so. The failure is handed to `onError` rather than swallowed —
 * a pin that silently does nothing is the one thing this control must
 * not be.
 *
 * Starts unpinned until the read completes. A pressed button over a
 * window that is not pinned, even for one frame, is the wrong
 * direction to be wrong in.
 */
export function useWindowPin(onError: (e: unknown) => void): {
  pinned: boolean;
  toggle: () => void;
} {
  const [pinned, setPinned] = useState(false);
  const onErrorRef = useRef(onError);
  onErrorRef.current = onError;

  useTauriEvent<Preferences>("cp-prefs-changed", (ev) => {
    setPinned(ev.payload.window_always_on_top);
  });

  useEffect(() => {
    let cancelled = false;
    api
      .preferencesGet()
      .then((p) => {
        if (!cancelled) setPinned(p.window_always_on_top);
      })
      .catch(() => {
        // Outside the webview, or a locked-up backend: stay unpinned,
        // which is what the window is.
      });
    return () => {
      cancelled = true;
    };
  }, []);

  // Read `pinned` from the closure rather than inside a state updater:
  // StrictMode double-invokes updaters, and an IPC call inside one
  // would fire twice (useToasts records the same trap for commits).
  const toggle = useCallback(() => {
    const next = !pinned;
    setPinned(next);
    api
      .preferencesSetWindowAlwaysOnTop(next)
      .then((p) => setPinned(p.window_always_on_top))
      .catch((e: unknown) => {
        setPinned(!next);
        onErrorRef.current(e);
      });
  }, [pinned]);

  return { pinned, toggle };
}
