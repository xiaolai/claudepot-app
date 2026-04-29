import { useEffect, useRef } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { api } from "../../api";
import { dispatchOsNotification } from "../../lib/notify";
import type { LiveSessionSummary, Preferences } from "../../types";

/**
 * `useCardNotifications` — fires native OS notifications when the
 * activity classifier emits a `CardEmitted` delta with severity
 * Warn or above.
 *
 * Coalescing now lives in the shared dispatcher (`lib/notify.ts`).
 * The bucket key `card:<kind>:<title>` matches the previous per-
 * (kind+title) grouping, with the same default of 3 dispatches per
 * 60s window — once that's exhausted, the bucket silently absorbs
 * further hits until the window rolls over. Replaces the
 * hand-rolled "≥3 in 60s → fire summary, reset" logic that
 * duplicated state across this hook and `useActivityNotifications`.
 *
 * Gated on the existing `notify_on_error` preference. Adding a
 * dedicated `notify_on_card` toggle later is a Settings change;
 * the JS wiring stays the same.
 *
 * The hook subscribes to live sessions on demand by mirroring
 * the aggregate `live-all` channel — every session that appears
 * gets a `live::<sid>` subscription opened (idempotent on the
 * Rust side); per-session `CardEmitted` deltas land here.
 */
export function useCardNotifications() {
  // Track which sessions we've subscribed to. The Rust-side
  // subscribe call is idempotent, but holding a JS-side listener
  // per session keeps the unsubscribe story clean.
  const subscriptions = useRef<Map<string, UnlistenFn>>(new Map());
  const enabledRef = useRef(false);

  // Fetch preference once on mount + listen for changes via the
  // cp-prefs-changed event whose payload IS the new Preferences. No
  // second preferencesGet() round-trip on each event, no ordering
  // race between back-to-back setters. Default-off until we hear
  // from the backend (fail-closed: no notifications without consent).
  useEffect(() => {
    // active-flag pattern: cleanup may run before listen() resolves
    // (StrictMode double-mount, fast unmount). Without the flag the
    // returned unlisten gets stashed into a stale closure and the
    // listener leaks for the page lifetime.
    let active = true;
    let aliveU: UnlistenFn | null = null;
    void api
      .preferencesGet()
      .then((p) => {
        if (active) enabledRef.current = !!p.notify_on_error;
      })
      .catch(() => {
        /* non-tauri env */
      });
    void listen<Preferences>("cp-prefs-changed", (ev) => {
      if (active && ev.payload) {
        enabledRef.current = !!ev.payload.notify_on_error;
      }
    })
      .then((u) => {
        if (!active) u();
        else aliveU = u;
      })
      .catch(() => {
        /* non-tauri env */
      });
    return () => {
      active = false;
      if (aliveU) aliveU();
    };
  }, []);

  useEffect(() => {
    // Mount-guard for the whole bootstrap chain. Cleanup may run
    // partway through sessionLiveSnapshot / subscribeNew /
    // listen("live-all"). Without `active`, any of those can finish
    // after teardown and silently attach a listener no one will ever
    // release.
    //
    // Permission probing previously ran here (and prompted on mount
    // even before the user enabled `notify_on_error`). It now lives
    // in `lib/notify.ts` as a lazy singleton — `dispatchOsNotification`
    // probes on first use and prompts only when an actual trigger
    // fires. That removes the unconditional first-mount prompt.
    let active = true;
    let aliveUnlisten: UnlistenFn | null = null;

    async function subscribeNew(sessions: LiveSessionSummary[]) {
      if (!active) return;
      const live = new Set(sessions.map((s) => s.session_id));
      // Subscribe in parallel — N live sessions used to serialize 2 N
      // IPC round-trips before the first delta could arrive. The
      // post-await race-checks guard against a concurrent subscribeNew
      // (e.g. the bootstrap call + a `live-all` event landing during
      // it) wiring the same session twice.
      const fresh = sessions.filter(
        (s) => !subscriptions.current.has(s.session_id),
      );
      await Promise.all(
        fresh.map(async (s) => {
          try {
            await api.sessionLiveSubscribe(s.session_id);
          } catch {
            return;
          }
          if (!active || subscriptions.current.has(s.session_id)) {
            // Teardown raced us, or another concurrent subscribeNew
            // already wired this session. Backend now holds the
            // singleton sub for s — paired unsubscribe so the next
            // mount can re-subscribe without AlreadySubscribed.
            void api.sessionLiveUnsubscribe(s.session_id).catch(() => {});
            return;
          }
          const unlisten = await listen(
            `live::${s.session_id}`,
            (ev) => handleDelta(ev.payload as LiveDeltaWire),
          );
          if (!active || subscriptions.current.has(s.session_id)) {
            // Same race outcomes as above — drop our frontend
            // unlisten AND release the backend forwarder.
            try {
              unlisten();
            } catch {
              /* ignore */
            }
            void api.sessionLiveUnsubscribe(s.session_id).catch(() => {});
            return;
          }
          subscriptions.current.set(s.session_id, unlisten);
        }),
      );
      // Unsub gone — pair the frontend unlisten with the backend
      // sessionLiveUnsubscribe so the rust forwarder doesn't keep
      // running and a future re-subscribe doesn't fail with
      // AlreadySubscribed (see api/activity.ts:93-98).
      for (const [sid, unlisten] of Array.from(subscriptions.current.entries())) {
        if (!live.has(sid)) {
          try {
            unlisten();
          } catch {
            /* ignore */
          }
          void api.sessionLiveUnsubscribe(sid).catch(() => {});
          subscriptions.current.delete(sid);
        }
      }
    }

    function handleDelta(payload: LiveDeltaWire) {
      if (!enabledRef.current) return;
      if (payload.kind !== "card_emitted") return;
      const card = payload as unknown as CardEmittedWire;
      const sev = card.severity;
      // Only Warn+ becomes a notification — Info / Notice stays
      // visible in the Events surface but doesn't push.
      if (sev !== "WARN" && sev !== "ERROR") return;

      // dedupeKey grain: kind+title — identical failures produce
      // identical titles (e.g. "Hook failed: PostToolUse:Edit") so
      // they land in the same bucket and stop after `maxBurst`.
      // group: full cwd — macOS threads notifications about the
      // same project regardless of which template fired, which is
      // what users actually care about ("what's wrong in foo?").
      // Using the full cwd (not the basename) prevents two different
      // projects with the same basename (e.g. ~/work/foo and
      // ~/personal/foo) from threading into one banner.
      const project = shortCwd(card.cwd);
      void dispatchOsNotification(card.title, project, {
        dedupeKey: `card:${card.card_kind}::${card.title}`,
        group: `project:${card.cwd}`,
        sound: "default",
      });
    }

    void (async () => {
      if (!active) return;
      // Bootstrap: subscribe to whatever's live right now.
      try {
        const initial = await api.sessionLiveSnapshot();
        if (!active) return;
        await subscribeNew(initial);
      } catch {
        /* no-tauri env */
      }
      if (!active) return;
      // Track session-list changes via the existing aggregate
      // channel. Payload: `LiveSessionSummary[]`.
      try {
        const fn = await listen<LiveSessionSummary[]>(
          "live-all",
          (ev) => {
            if (active) void subscribeNew(ev.payload ?? []);
          },
        );
        if (active) aliveUnlisten = fn;
        else fn();
      } catch {
        /* ignore */
      }
    })();

    const sessionsRef = subscriptions.current;
    return () => {
      active = false;
      if (aliveUnlisten) aliveUnlisten();
      // Pair every frontend unlisten with the backend unsubscribe
      // so future mounts can re-subscribe (single-subscriber
      // contract on the rust side). Fire-and-forget — the backend
      // already swallows unknown-session unsubscribes.
      for (const [sid, u] of sessionsRef) {
        try {
          u();
        } catch {
          /* ignore */
        }
        void api.sessionLiveUnsubscribe(sid).catch(() => {});
      }
      sessionsRef.clear();
      // Bucket state used to be cleared here; it now lives in the
      // shared dispatcher's module-level Map and self-evicts on
      // window expiry.
    };
  }, []);
}

function shortCwd(cwd: string): string {
  const parts = cwd.split(/[\\/]/);
  return parts[parts.length - 1] || cwd;
}

// Mirror of the Rust `LiveDeltaKindDto` discriminator. Kept narrow
// to just what this hook actually inspects — adding fields here
// without using them would be dead code.
interface LiveDeltaWire {
  kind: string;
  card_kind?: string;
  severity?: string;
  title?: string;
  cwd?: string;
  // intentionally `string` for the kind branches we don't care about;
  // the discriminator check above narrows.
  [k: string]: unknown;
}

// Discriminated narrowing helper for the only branch we react to.
type CardEmittedWire = {
  kind: "card_emitted";
  id: number;
  card_kind: string;
  severity: "INFO" | "NOTICE" | "WARN" | "ERROR";
  title: string;
  ts_ms: number;
  plugin?: string;
  cwd: string;
};
const _typeCheck: CardEmittedWire | undefined = undefined;
void _typeCheck;
