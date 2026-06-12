import { useEffect, useRef } from "react";
import {
  listen,
  type Event as TauriEvent,
  type UnlistenFn,
} from "@tauri-apps/api/event";

/**
 * Distinguish "this renderer isn't running under Tauri" (vitest under
 * jsdom, a future preview build — expected, skip silently) from a
 * real `listen()` failure (renderer reload race mid-mount, a renamed
 * channel, an unexpected throw inside the plugin's invoke layer —
 * must be diagnosable). A non-Tauri renderer raises a failure whose
 * message mentions `__TAURI_*` or a missing tauri binding; everything
 * else gets a `console.warn` so a channel typo doesn't silently
 * become a "missing toast" mystery.
 */
function isNonTauriEnvError(err: unknown): boolean {
  const msg = err instanceof Error ? err.message : String(err);
  return (
    /__TAURI/i.test(msg) ||
    (/tauri/i.test(msg) && /undefined|not (a )?function/i.test(msg))
  );
}

/**
 * Subscribe to a Tauri event channel for the component's lifetime.
 * Pass `null` to skip subscription (useful when the channel name is
 * derived from state that hasn't resolved yet, e.g. an op-id not yet
 * returned by the backend).
 *
 * The handler is held in a ref, so an unstable handler identity does
 * NOT resubscribe — the subscription lives for as long as `channel`
 * is unchanged, and every event sees the latest handler. (The old
 * `[channel, handler]` deps re-subscribed on every handler change,
 * opening a window between cleanup and the next async `listen()`
 * resolution where an event could land unobserved — which pushed
 * callers toward hand-rolled `listen()` + refs.)
 */
export function useTauriEvent<T>(
  channel: string | null,
  handler: (event: TauriEvent<T>) => void,
): void {
  const handlerRef = useRef(handler);
  handlerRef.current = handler;

  useEffect(() => {
    if (!channel) return;
    let unlisten: UnlistenFn | undefined;
    let cancelled = false;

    listen<T>(channel, (event) => {
      // Guard the gap between cleanup and an in-flight delivery —
      // mirrors the `active` flag the hand-rolled sites used.
      if (!cancelled) handlerRef.current(event);
    })
      .then((fn) => {
        if (cancelled) {
          // Component unmounted before listen() resolved — drop immediately.
          fn();
        } else {
          unlisten = fn;
        }
      })
      .catch((err) => {
        if (!isNonTauriEnvError(err)) {
          // eslint-disable-next-line no-console
          console.warn(`useTauriEvent: listen(${channel}) failed`, err);
        }
      });

    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  }, [channel]);
}

/**
 * Multi-channel variant of `useTauriEvent` — one subscription per
 * entry in `handlers`, all torn down together on unmount. Absorbs
 * the `wire<T>(channel, handler)` helper that used to be copy-pasted
 * across useRotationEvents / useBackgroundChangeEmits /
 * useAgentEventToasts.
 *
 * Handlers are held in a ref (same no-resubscribe contract as
 * `useTauriEvent`); the subscription set re-wires only when the
 * channel NAMES change. Pass `null` to skip subscription entirely.
 *
 * Handlers receive the raw `TauriEvent` — callers own their payload
 * guard (`if (!ev.payload) return;`) so empty/dropped payload policy
 * stays at the call site.
 */
export function useTauriEvents(
  handlers: Record<string, (event: TauriEvent<never>) => void> | null,
): void {
  const handlersRef = useRef(handlers);
  handlersRef.current = handlers;

  // Stable dep derived from the channel names alone. Tauri validates
  // event names against `[a-zA-Z0-9/:_-]`, so a space separator is
  // collision-free.
  const channelsKey =
    handlers === null ? null : Object.keys(handlers).join(" ");

  useEffect(() => {
    if (channelsKey === null || channelsKey === "") return;
    let cancelled = false;
    const unlisteners: UnlistenFn[] = [];

    for (const channel of channelsKey.split(" ")) {
      listen<unknown>(channel, (event) => {
        if (cancelled) return;
        handlersRef.current?.[channel]?.(event as TauriEvent<never>);
      })
        .then((fn) => {
          if (cancelled) fn();
          else unlisteners.push(fn);
        })
        .catch((err) => {
          if (!isNonTauriEnvError(err)) {
            // eslint-disable-next-line no-console
            console.warn(`useTauriEvents: listen(${channel}) failed`, err);
          }
        });
    }

    return () => {
      cancelled = true;
      unlisteners.forEach((fn) => fn());
    };
  }, [channelsKey]);
}
