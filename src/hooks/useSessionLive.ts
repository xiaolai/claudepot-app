import { useEffect, useSyncExternalStore } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { api } from "../api";
import type { LiveSessionSummary } from "../types";

/**
 * `useSessionLive` — subscribe to the live aggregate session list.
 *
 * Contract:
 *   * On first mount, hydrates via `api.sessionLiveSnapshot()` so
 *     surfaces render immediately without waiting for the first
 *     event.
 *   * Subscribes to the `live-all` Tauri channel; every event
 *     replaces the local snapshot. The backend publishes idempotent
 *     full lists (last-writer-wins), so missed intermediate events
 *     are never observable.
 *   * Commits to React at most once per animation frame — even when
 *     the backend bursts events, the hook coalesces to ~60 Hz max
 *     via `requestAnimationFrame`. This preserves the plan's
 *     "honor human pace" rule and prevents re-render storms.
 *
 * One module-scoped store is shared by every caller — subscribing
 * from N components costs one backend listener, not N.
 */

type Listener = () => void;

let snapshot: LiveSessionSummary[] = [];
let pending: LiveSessionSummary[] | null = null;
let rafHandle: number | null = null;
const listeners = new Set<Listener>();
let hydrated = false;
let unlistenEvent: UnlistenFn | null = null;
let unlistenPromise: Promise<UnlistenFn> | null = null;

/** Apply a new value to the store, coalesced to one RAF tick. */
function scheduleCommit(next: LiveSessionSummary[]): void {
  pending = next;
  if (rafHandle !== null) return;
  rafHandle = requestAnimationFrame(() => {
    rafHandle = null;
    if (pending !== null) {
      snapshot = pending;
      pending = null;
      listeners.forEach((l) => l());
    }
  });
}

/** React's useSyncExternalStore subscribe callback. */
function subscribe(listener: Listener): () => void {
  listeners.add(listener);
  if (listeners.size === 1) {
    // First subscriber — wire up the backend bridge lazily so tests
    // that import this module without a Tauri environment don't
    // pay the listen() cost.
    wireBackend();
  }
  return () => {
    listeners.delete(listener);
    if (listeners.size === 0) {
      teardownBackend();
    }
  };
}

function getSnapshot(): LiveSessionSummary[] {
  return snapshot;
}

function wireBackend(): void {
  // Hydrate synchronously-ish: fire off the snapshot call; commit on
  // resolve. The backend returns an empty list if the runtime isn't
  // running, so this is safe before consent — no files are touched.
  if (!hydrated) {
    api
      .sessionLiveSnapshot()
      .then((list) => {
        hydrated = true;
        scheduleCommit(list);
      })
      .catch(() => {
        // Runtime not started yet, or no Tauri env.
      });
  }
  // Subscribe to the aggregate channel so any updates the
  // backend emits (once it's started via the consent-driven path
  // in App.tsx) reach us without re-wiring. DO NOT call
  // sessionLiveStart() here — the runtime must only start after
  // the user has accepted the consent modal (or opted in
  // previously). Auto-starting on first subscriber bypasses the
  // trust boundary.
  //
  // Guard against the StrictMode double-mount race: if listen()
  // resolves AFTER teardownBackend() has already run, we'd leak
  // a listener. Track the in-flight promise and null it out on
  // teardown; resolve the returned unlisten fn only if the
  // promise is still "current."
  const current = listen<LiveSessionSummary[]>("live-all", (event) => {
    scheduleCommit(event.payload);
  });
  unlistenPromise = current;
  current
    .then((fn) => {
      if (unlistenPromise !== current) {
        // Teardown raced — drop the listener immediately.
        fn();
      } else {
        unlistenEvent = fn;
      }
    })
    .catch(() => {
      // Non-Tauri env — swallow.
    });
}

function teardownBackend(): void {
  if (unlistenEvent) {
    unlistenEvent();
    unlistenEvent = null;
  }
  // Null out the in-flight promise so any still-pending listen()
  // resolve recognizes it as stale and drops its own fn.
  unlistenPromise = null;
  if (rafHandle !== null) {
    cancelAnimationFrame(rafHandle);
    rafHandle = null;
  }
  pending = null;
  // A full stop (session_live_stop) happens via the App-level
  // lifecycle hook only — the hook never calls it on its own.
}

/** Returns the current live session list, re-rendering the caller
 *  whenever the list changes (coalesced to one RAF tick). */
export function useSessionLive(): LiveSessionSummary[] {
  // Allocate a stable empty array for SSR / no-tauri environments.
  return useSyncExternalStore(
    subscribe,
    getSnapshot,
    EMPTY_SERVER_SNAPSHOT,
  );
}

const EMPTY: LiveSessionSummary[] = [];
const EMPTY_SERVER_SNAPSHOT = (): LiveSessionSummary[] => EMPTY;

/** Lifecycle hook — no-op today; App.tsx owns the
 *  sessionLiveStop call path on unmount when applicable. Kept as a
 *  named export so consumer components don't need to rethread
 *  imports if we later restore the auto-stop behavior. */
export function useSessionLiveLifecycle(): void {
  useEffect(() => undefined, []);
}

/** Test-only: reset the module-scoped store. Not exported from
 *  the public API, but available to vitest suites that `import *
 *  as hookModule` to reach it. */
export function __resetForTests(): void {
  snapshot = [];
  pending = null;
  if (rafHandle !== null) {
    cancelAnimationFrame(rafHandle);
    rafHandle = null;
  }
  listeners.clear();
  hydrated = false;
  unlistenEvent = null;
  unlistenPromise = null;
}
