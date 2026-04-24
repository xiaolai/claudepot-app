import { useEffect } from "react";
import { listen, type Event as TauriEvent, type UnlistenFn } from "@tauri-apps/api/event";

/**
 * Subscribe to a Tauri event channel for the component's lifetime.
 * Pass `null` to skip subscription (useful when the channel name is
 * derived from state that hasn't resolved yet, e.g. an op-id not yet
 * returned by the backend).
 *
 * The handler is captured by effect dependency, so stable handlers
 * avoid resubscribing on every render. Callers that need fresh state
 * inside the handler should wrap with `useCallback` or use a ref.
 */
export function useTauriEvent<T>(
  channel: string | null,
  handler: (event: TauriEvent<T>) => void,
): void {
  useEffect(() => {
    if (!channel) return;
    let unlisten: UnlistenFn | undefined;
    let cancelled = false;

    listen<T>(channel, handler)
      .then((fn) => {
        if (cancelled) {
          // Component unmounted before listen() resolved — drop immediately.
          fn();
        } else {
          unlisten = fn;
        }
      })
      .catch((err) => {
        // In non-Tauri environments (e.g. unit tests without a mock)
        // listen may reject; swallow quietly so the app still renders.
        // Real Tauri subscription failures are rare and will surface
        // elsewhere (event never fires → op never completes → user
        // sees the RunningOpStrip stall).
        if (import.meta.env.DEV) {
          // eslint-disable-next-line no-console
          console.warn(`useTauriEvent: listen(${channel}) failed`, err);
        }
      });

    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  }, [channel, handler]);
}
