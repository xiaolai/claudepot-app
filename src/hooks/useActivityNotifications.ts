import { useEffect, useRef, useState } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { api } from "../api";
import { dispatchOsNotification } from "../lib/notify";
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

  // Permission probing now lives in `lib/notify.ts` as a singleton —
  // shared across this hook, useCardNotifications, useOpDoneNotifications,
  // and the Settings "Send test" / "Request" buttons. One probe state,
  // one prompt, one focus gate.

  // Load prefs once on mount; subsequent updates ride on the
  // `cp-prefs-changed` event whose payload IS the new Preferences
  // snapshot — no second preferencesGet() and no ordering race
  // between back-to-back setters.
  useEffect(() => {
    let cancelled = false;
    let unlisten: UnlistenFn | null = null;

    const applyPrefs = (p: Preferences) => {
      if (cancelled) return;
      prefsRef.current = p;
      setPrefsVersion((v) => v + 1);
    };

    api
      .preferencesGet()
      .then(applyPrefs)
      .catch(() => {
        /* no-tauri env */
      });
    listen<Preferences>("cp-prefs-changed", (ev) => {
      if (ev.payload) applyPrefs(ev.payload);
    })
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      })
      .catch(() => {
        /* no-tauri env */
      });
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);

  useEffect(() => {
    const prefs = prefsRef.current;
    if (!prefs) return;
    const memo = memoRef.current;
    const now = Date.now();

    /** Fire an OS notification for the given transition. The shared
     *  dispatcher applies the focus gate, the permission probe, AND
     *  the unified token-bucket coalescing — per-session+kind keys
     *  let one busy session burn its tokens without starving another.
     *  `group` threads OS notifications by session so the user gets
     *  one expandable banner per project rather than a stack of
     *  five lookalike toasts. */
    const dispatch = (
      sessionId: string,
      cwd: string,
      kind: "error" | "stuck" | "idle-done",
      title: string,
      body: string,
    ) => {
      // Intent: send the user back to wherever `claude` is running.
      // The shell-level focus consumer translates this to a Tauri
      // command that walks the session's process tree and activates
      // the host terminal/editor; falls back to opening the
      // transcript in Claudepot's Projects pane when the host can't
      // be resolved.
      void dispatchOsNotification(title, body, {
        dedupeKey: `session:${sessionId}:${kind}`,
        group: `session:${sessionId}`,
        sound: "default",
        target: { kind: "host", session_id: sessionId, cwd },
      });
    };

    // Pre-compute disambiguated labels so notifications for two
    // sibling projects with the same basename (e.g. ~/work/foo vs
    // ~/personal/foo) don't render an identical title in macOS
    // Notification Center. Pure basename when unique, parent/basename
    // when colliding — minimum extra noise for unambiguous projects,
    // unambiguous-by-construction when it matters.
    const labels = projectLabels(sessions.map((s) => s.cwd));

    const seen = new Set<string>();
    for (const s of sessions) {
      seen.add(s.session_id);
      const prev = memo.get(s.session_id);
      const busyStartedMs =
        s.status === "busy"
          ? (prev?.busyStartedMs ?? now)
          : null;

      const project = labels.get(s.cwd) ?? projectBasename(s.cwd);

      // Error-burst transition. Coalescing now lives in the shared
      // dispatcher's token bucket — the per-session canFire check
      // that used to live here was a hand-rolled rate-limit; one
      // policy is easier to reason about than three.
      if (
        prefs.notify_on_error &&
        s.errored &&
        !(prev?.lastErrored ?? false)
      ) {
        dispatch(s.session_id, s.cwd, "error", project, "multiple errors in the last minute");
      }

      // Stuck transition
      if (
        prefs.notify_on_stuck_minutes != null &&
        s.stuck &&
        !(prev?.lastStuck ?? false)
      ) {
        dispatch(
          s.session_id,
          s.cwd,
          "stuck",
          project,
          `possibly stuck (tool call > ${prefs.notify_on_stuck_minutes ?? 10} min)`,
        );
      }

      // Idle-after-work transition
      if (
        prefs.notify_on_idle_done &&
        s.status === "idle" &&
        prev?.lastStatus === "busy" &&
        prev.busyStartedMs != null &&
        now - prev.busyStartedMs >= 120_000
      ) {
        const minutes = Math.floor((now - prev.busyStartedMs) / 60_000);
        dispatch(s.session_id, s.cwd, "idle-done", project, `done (${minutes}m)`);
      }

      // Refresh the memo so the next tick can diff against current
      // state. Single update site keeps lastErrored / lastStuck /
      // status/busyStartedMs in lockstep.
      memo.set(s.session_id, nextMemo(s, busyStartedMs));
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
): SessionMemo {
  return {
    lastStatus: s.status,
    lastErrored: s.errored,
    lastStuck: s.stuck,
    busyStartedMs,
  };
}

export function projectBasename(cwd: string): string {
  const trimmed = cwd.replace(/[/\\]+$/, "");
  if (!trimmed) return cwd;
  const idx = Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\"));
  const base = idx >= 0 ? trimmed.slice(idx + 1) : trimmed;
  return base || trimmed;
}

/** Split `cwd` into its non-empty path components, separator-agnostic.
 *  Drops any trailing separator, then splits on `/` or `\` greedily.
 *  Used by `projectLabels` for collision detection. */
function pathParts(cwd: string): string[] {
  const trimmed = cwd.replace(/[/\\]+$/, "");
  if (!trimmed) return [];
  return trimmed.split(/[/\\]+/).filter((p) => p.length > 0);
}

/** Build a `cwd → human label` map that disambiguates same-basename
 *  collisions. Pure basename for unique projects (one-word, scannable
 *  in macOS Notification Center); `parent/basename` only for those
 *  that would otherwise collide. Pure function — no side effects, no
 *  dependence on render order — so swapping in `projectLabels` for
 *  ad-hoc `projectBasename` calls is a drop-in.
 *
 *  Worth disambiguating because two sessions in `~/work/foo` and
 *  `~/personal/foo` would render identical titles, and macOS stacks
 *  them by `threadId` which makes the basename collision invisible
 *  until the user expands the stack. */
export function projectLabels(cwds: string[]): Map<string, string> {
  const result = new Map<string, string>();
  // First pass: count basename occurrences across the input set.
  const counts = new Map<string, number>();
  for (const cwd of cwds) {
    const base = projectBasename(cwd);
    counts.set(base, (counts.get(base) ?? 0) + 1);
  }
  // Second pass: emit labels, prepending the parent for collisions.
  for (const cwd of cwds) {
    if (result.has(cwd)) continue;
    const parts = pathParts(cwd);
    if (parts.length === 0) {
      result.set(cwd, cwd);
      continue;
    }
    const base = parts[parts.length - 1];
    const collides = (counts.get(base) ?? 0) > 1;
    if (!collides || parts.length < 2) {
      result.set(cwd, base);
    } else {
      result.set(cwd, `${parts[parts.length - 2]}/${base}`);
    }
  }
  return result;
}
