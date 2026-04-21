import { useEffect, useRef } from "react";
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";
import { api } from "../api";
import { useSessionLive } from "./useSessionLive";
import type { LiveSessionSummary, Preferences } from "../types";

/**
 * `useActivityNotifications` — observe aggregate transitions and
 * surface in-app toasts for the user-enabled trigger classes.
 *
 * Detection rules:
 *   * `on_error`: `errored` overlay flipped false → true on any
 *     session. One toast per session per 60 s, hard-capped.
 *   * `on_idle_done`: a session that had been `busy` for ≥ 2 min
 *     transitioned to `idle`. Same 60-s-per-session rate limit.
 *   * `on_stuck_minutes`: `stuck` overlay fires when a tool call
 *     has been open longer than the configured threshold. Backend
 *     already computes the `stuck` bool using its own threshold
 *     (STUCK_THRESHOLD = 10 min); this toast fires on the false →
 *     true transition. Pref is Option<u32> meaning "enable when
 *     value is set"; the threshold number itself is advisory and
 *     only used in the toast copy, not for client-side detection.
 *
 * Uses the existing useSessionLive aggregate snapshot — no extra
 * backend round-trip. A per-render ref tracks the previous state
 * keyed by session_id so we can compute transitions without fighting
 * React's render semantics.
 */

/** Matches the shape of `useToasts().pushToast` — two-kind palette
 *  (info / error). Activity uses `info` for successful transitions
 *  (done, idle-after-work) and `error` for the alerting states
 *  (errored burst, stuck tool call). */
export type ToastPusher = (
  kind: "info" | "error",
  text: string,
  onUndo?: () => void,
  opts?: { dedupeKey?: string },
) => void;

interface SessionMemo {
  lastStatus: LiveSessionSummary["status"];
  lastErrored: boolean;
  lastStuck: boolean;
  busyStartedMs: number | null;
  lastFiredMs: number;
}

export function useActivityNotifications(pushToast: ToastPusher): void {
  const sessions = useSessionLive();
  const memoRef = useRef(new Map<string, SessionMemo>());
  const prefsRef = useRef<Preferences | null>(null);
  const osPermissionRef = useRef<"granted" | "denied" | "unknown">(
    "unknown",
  );

  // Load prefs once + refresh every 10s. The triggers are a user
  // configuration; getting a fresh read once per 10s is far cheaper
  // than round-tripping on every tick, and the feature is itself
  // opt-in so misses during the refresh window are harmless.
  useEffect(() => {
    let cancelled = false;
    const load = () => {
      api
        .preferencesGet()
        .then(async (p) => {
          if (cancelled) return;
          prefsRef.current = p;
          // Probe OS-notification permission the first time any
          // notification pref is flipped on. No request until a
          // trigger actually fires, so a cautious user who never
          // enables alerts never sees the OS prompt.
          const wantsOs =
            p.notify_on_error ||
            p.notify_on_idle_done ||
            p.notify_on_stuck_minutes != null ||
            p.notify_on_spend_usd != null;
          if (wantsOs && osPermissionRef.current === "unknown") {
            try {
              const granted = await isPermissionGranted();
              osPermissionRef.current = granted ? "granted" : "denied";
            } catch {
              osPermissionRef.current = "denied";
            }
          }
        })
        .catch(() => {
          /* no-tauri env */
        });
    };
    load();
    const id = setInterval(load, 10_000);
    return () => {
      cancelled = true;
      clearInterval(id);
    };
  }, []);

  useEffect(() => {
    const prefs = prefsRef.current;
    if (!prefs) return;
    const memo = memoRef.current;
    const now = Date.now();
    const RATE_LIMIT_MS = 60_000;

    /** Fire both the in-app toast and (if the user has OS
     *  permission) a system notification. The OS path is
     *  fire-and-forget; any error is swallowed so a denied
     *  permission never breaks the in-app toast flow. */
    const alert = (
      kind: "info" | "error",
      title: string,
      body: string,
      dedupeKey: string,
    ) => {
      pushToast(kind, `${title} — ${body}`, undefined, { dedupeKey });
      if (osPermissionRef.current === "granted") {
        try {
          sendNotification({ title, body });
        } catch {
          /* swallow */
        }
      } else if (osPermissionRef.current === "unknown") {
        // Ask on-demand at the first trigger, since the initial
        // pref load may not have reached this code yet.
        requestPermission()
          .then((perm) => {
            osPermissionRef.current =
              perm === "granted" ? "granted" : "denied";
            if (perm === "granted") {
              try {
                sendNotification({ title, body });
              } catch {
                /* swallow */
              }
            }
          })
          .catch(() => {
            osPermissionRef.current = "denied";
          });
      }
    };

    const seen = new Set<string>();
    for (const s of sessions) {
      seen.add(s.session_id);
      const prev = memo.get(s.session_id);
      const busyStartedMs =
        s.status === "busy"
          ? (prev?.busyStartedMs ?? now)
          : null;

      const canFire = (prev?.lastFiredMs ?? 0) + RATE_LIMIT_MS <= now;

      const project = projectBasename(s.cwd);

      // Error-burst transition
      if (
        prefs.notify_on_error &&
        s.errored &&
        !(prev?.lastErrored ?? false) &&
        canFire
      ) {
        alert(
          "error",
          project,
          "multiple errors in the last minute",
          `activity-error-${s.session_id}`,
        );
        memo.set(s.session_id, nextMemo(s, busyStartedMs, now));
        continue;
      }

      // Stuck transition
      if (
        prefs.notify_on_stuck_minutes != null &&
        s.stuck &&
        !(prev?.lastStuck ?? false) &&
        canFire
      ) {
        alert(
          "error",
          project,
          "possibly stuck (tool call > 10 min)",
          `activity-stuck-${s.session_id}`,
        );
        memo.set(s.session_id, nextMemo(s, busyStartedMs, now));
        continue;
      }

      // Idle-after-work transition
      if (
        prefs.notify_on_idle_done &&
        s.status === "idle" &&
        prev?.lastStatus === "busy" &&
        prev.busyStartedMs != null &&
        now - prev.busyStartedMs >= 120_000 &&
        canFire
      ) {
        const minutes = Math.floor((now - prev.busyStartedMs) / 60_000);
        alert(
          "info",
          project,
          `done (${minutes}m)`,
          `activity-idle-${s.session_id}`,
        );
        memo.set(s.session_id, nextMemo(s, busyStartedMs, now));
        continue;
      }

      // No transition fired — just refresh the memo so next tick
      // has the current state to diff against.
      memo.set(s.session_id, nextMemo(s, busyStartedMs, prev?.lastFiredMs ?? 0));
    }

    // Reap memo entries for sessions that dropped off the list so
    // the map doesn't grow forever.
    for (const id of [...memo.keys()]) {
      if (!seen.has(id)) memo.delete(id);
    }
  }, [sessions, pushToast]);
}

function nextMemo(
  s: LiveSessionSummary,
  busyStartedMs: number | null,
  lastFiredMs: number,
): SessionMemo {
  return {
    lastStatus: s.status,
    lastErrored: s.errored,
    lastStuck: s.stuck,
    busyStartedMs,
    lastFiredMs,
  };
}

function projectBasename(cwd: string): string {
  const trimmed = cwd.replace(/\/+$/, "");
  const idx = trimmed.lastIndexOf("/");
  return idx >= 0 ? trimmed.slice(idx + 1) : trimmed;
}
