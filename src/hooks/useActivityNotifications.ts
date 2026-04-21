import { useEffect, useRef, useState } from "react";
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
 * fire OS notifications for user-enabled trigger classes.
 *
 * Detection rules:
 *   * `on_error`: `errored` overlay flipped false → true on any
 *     session. One notification per session per 60 s, hard-capped.
 *   * `on_idle_done`: a session that had been `busy` for ≥ 2 min
 *     transitioned to `idle`. Same 60-s-per-session rate limit.
 *   * `on_stuck_minutes`: `stuck` overlay fires when a tool call
 *     has been open longer than the configured threshold. Backend
 *     already computes the `stuck` bool using its own threshold
 *     (STUCK_THRESHOLD = 10 min); this notification fires on the
 *     false → true transition. Pref is Option<u32> meaning "enable
 *     when value is set"; the threshold number itself is advisory
 *     and only used in the notification copy, not for client-side
 *     detection.
 *
 * In-app signal lives on the session row itself (errored border +
 * tag) and the Activity nav badge — not in ephemeral toasts.
 * Toasts are reserved for user-action acknowledgements.
 *
 * Uses the existing useSessionLive aggregate snapshot — no extra
 * backend round-trip. A per-render ref tracks the previous state
 * keyed by session_id so we can compute transitions without fighting
 * React's render semantics.
 */

interface SessionMemo {
  lastStatus: LiveSessionSummary["status"];
  lastErrored: boolean;
  lastStuck: boolean;
  busyStartedMs: number | null;
  lastFiredMs: number;
}

/** Returns the count of sessions currently in an alerting state
 *  (errored or stuck). Used by AppShell to drive the Activity nav
 *  badge without a second useSessionLive subscription at shell level. */
export function useActivityNotifications(): number {
  const sessions = useSessionLive();
  const memoRef = useRef(new Map<string, SessionMemo>());
  const prefsRef = useRef<Preferences | null>(null);
  // Bumped on every successful preferencesGet() resolve so the
  // notification effect re-runs when prefs first load — without this,
  // transitions that arrive before the first prefsGet round-trip are
  // silently dropped because the effect exits early on prefs === null.
  const [prefsVersion, setPrefsVersion] = useState(0);
  // Three-state permission machine:
  //   * "unknown": no probe yet — ask on next pref-enabled tick
  //   * "not-requested": probed isPermissionGranted, got false, but
  //     no requestPermission call has fired. First trigger will ask.
  //   * "granted" / "denied": terminal after requestPermission
  //     result. "denied" sticks for the session — we never
  //     re-prompt; user opts back in via System Settings.
  const osPermissionRef = useRef<
    "unknown" | "not-requested" | "granted" | "denied"
  >("unknown");

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
          setPrefsVersion((v) => v + 1);
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
              // If not yet granted, leave as "not-requested" so the
              // next trigger does a user-facing requestPermission.
              // isPermissionGranted returns false BEFORE any prompt
              // has been shown, so treating that as terminal
              // "denied" would silently suppress all future OS
              // notifications on a fresh install.
              osPermissionRef.current = granted
                ? "granted"
                : "not-requested";
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

    /** Fire an OS notification for the given transition.
     *  Fire-and-forget; any error is swallowed so a denied
     *  permission never interrupts the detection loop.
     *
     *  Named `dispatch` deliberately — `alert` would shadow
     *  `window.alert` and set a footgun for any future
     *  `globalThis.alert(...)` audit grep. */
    const dispatch = (title: string, body: string) => {
      if (osPermissionRef.current === "granted") {
        try {
          sendNotification({ title, body });
        } catch {
          /* swallow */
        }
      } else if (
        osPermissionRef.current === "unknown" ||
        osPermissionRef.current === "not-requested"
      ) {
        // First trigger after pref-enable: ask for permission. If
        // the user grants it, fire this alert AND every future
        // one. If they deny, stay denied for the session.
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
        dispatch(project, "multiple errors in the last minute");
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
        dispatch(project, `possibly stuck (tool call > ${prefs.notify_on_stuck_minutes ?? 10} min)`);
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
        dispatch(project, `done (${minutes}m)`);
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
  }, [sessions, prefsVersion]);

  return sessions.filter((s) => s.errored || s.stuck).length;
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

export function projectBasename(cwd: string): string {
  const trimmed = cwd.replace(/[/\\]+$/, "");
  if (!trimmed) return cwd;
  const idx = Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\"));
  const base = idx >= 0 ? trimmed.slice(idx + 1) : trimmed;
  return base || trimmed;
}
