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
let backendStarted = false;
let unlistenEvent: UnlistenFn | null = null;

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
  // resolve. If Tauri isn't available (unit test without mock),
  // catch and leave the store empty.
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
  // Start the backend runtime. Idempotent on the Rust side.
  if (!backendStarted) {
    backendStarted = true;
    api.sessionLiveStart().catch(() => {
      // Leave started=true so we don't thrash retry; the runtime
      // itself is idempotent and the user can retry via a refresh.
    });
  }
  // Subscribe to the aggregate channel.
  listen<LiveSessionSummary[]>("live-all", (event) => {
    scheduleCommit(event.payload);
  })
    .then((fn) => {
      unlistenEvent = fn;
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
  if (rafHandle !== null) {
    cancelAnimationFrame(rafHandle);
    rafHandle = null;
  }
  pending = null;
  // Intentionally leave `backendStarted = true`. The runtime is
  // cheap to keep alive between mounts; re-wiring on every
  // StrictMode double-mount in dev would thrash the poll task.
  // A full stop happens via `api.sessionLiveStop()`, which the
  // hook never calls on its own.
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

/** Lifecycle hook — calls `session_live_stop` when the host
 *  component unmounts. Use at the App root, not per-surface, so a
 *  feature teardown cascades cleanly. */
export function useSessionLiveLifecycle(): void {
  useEffect(() => {
    return () => {
      if (backendStarted) {
        api.sessionLiveStop().catch(() => {
          /* best-effort */
        });
        backendStarted = false;
      }
    };
  }, []);
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
  backendStarted = false;
  unlistenEvent = null;
}
