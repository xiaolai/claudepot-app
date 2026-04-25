import { useEffect, useRef } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";
import { api } from "../../api";
import type { LiveSessionSummary } from "../../types";

/**
 * `useCardNotifications` — fires native OS notifications when the
 * activity classifier emits a `CardEmitted` delta with severity
 * Warn or above.
 *
 * Coalescing rule (design v2 §8): ≥3 cards of the same template_id
 * within 60 seconds collapse into one notification with the count.
 * Prevents the "100 plugin_missing" notification storm when a
 * single broken hook fires repeatedly across a turn.
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
  // Coalescer state: per template_id, the timestamps of recent
  // notifications (within the 60s window). When ≥3 land in
  // the window, fire one summary notification and reset.
  const recent = useRef<Map<string, number[]>>(new Map());
  const COALESCE_WINDOW_MS = 60_000;
  const COALESCE_THRESHOLD = 3;

  // Fetch preference + listen for changes via the cp-prefs event
  // pattern other surfaces use. Default-off until we hear from the
  // backend (fail-closed: no notifications without consent).
  useEffect(() => {
    let aliveU: UnlistenFn | null = null;
    void api
      .preferencesGet()
      .then((p) => {
        enabledRef.current = !!p.notify_on_error;
      })
      .catch(() => {
        /* non-tauri env */
      });
    void listen("cp-prefs-changed", () => {
      void api
        .preferencesGet()
        .then((p) => {
          enabledRef.current = !!p.notify_on_error;
        })
        .catch(() => {});
    }).then((u) => {
      aliveU = u;
    });
    return () => {
      if (aliveU) aliveU();
    };
  }, []);

  useEffect(() => {
    let aliveUnlisten: UnlistenFn | null = null;
    let permissionGranted = false;

    async function ensurePermission() {
      try {
        permissionGranted = await isPermissionGranted();
        if (!permissionGranted) {
          const r = await requestPermission();
          permissionGranted = r === "granted";
        }
      } catch {
        permissionGranted = false;
      }
    }

    async function subscribeNew(sessions: LiveSessionSummary[]) {
      const live = new Set(sessions.map((s) => s.session_id));
      // Sub new
      for (const s of sessions) {
        if (subscriptions.current.has(s.session_id)) continue;
        try {
          await api.sessionLiveSubscribe(s.session_id);
        } catch {
          continue;
        }
        const unlisten = await listen(
          `live::${s.session_id}`,
          (ev) => handleDelta(ev.payload as LiveDeltaWire),
        );
        subscriptions.current.set(s.session_id, unlisten);
      }
      // Unsub gone
      for (const [sid, unlisten] of Array.from(subscriptions.current.entries())) {
        if (!live.has(sid)) {
          try {
            unlisten();
          } catch {
            /* ignore */
          }
          subscriptions.current.delete(sid);
        }
      }
    }

    function handleDelta(payload: LiveDeltaWire) {
      if (!enabledRef.current || !permissionGranted) return;
      if (payload.kind !== "card_emitted") return;
      const card = payload as unknown as CardEmittedWire;
      const sev = card.severity;
      // Only Warn+ becomes a notification — Info / Notice stays
      // visible in the Events surface but doesn't push.
      if (sev !== "WARN" && sev !== "ERROR") return;

      // Coalescing: track per (kind+title) bucket. CardEmitted
      // deltas don't carry the template_id directly, so title-as-key
      // is the right grain — identical failures produce identical
      // titles (e.g. "Hook failed: PostToolUse:Edit").
      const key = `${card.card_kind}::${card.title}`;
      const now = Date.now();
      const arr = recent.current.get(key) ?? [];
      const filtered = arr.filter((t) => now - t < COALESCE_WINDOW_MS);
      filtered.push(now);
      recent.current.set(key, filtered);

      if (filtered.length === COALESCE_THRESHOLD) {
        // Hit the threshold this exact tick — coalesce: one summary
        // notification, then reset the window so we don't double-fire.
        try {
          sendNotification({
            title: `${filtered.length}× ${card.title}`,
            body: `Repeated ${sev.toLowerCase()} in ${shortCwd(
              card.cwd,
            )}. Open Events to inspect.`,
          });
        } catch {
          /* ignore */
        }
        recent.current.set(key, []);
      } else if (filtered.length < COALESCE_THRESHOLD) {
        // Below threshold: emit each card individually.
        try {
          sendNotification({
            title: card.title,
            body: shortCwd(card.cwd),
          });
        } catch {
          /* ignore */
        }
      }
      // > threshold: silently absorb — the threshold-hit firing
      // already represents the burst. Window will roll over and
      // the next batch starts fresh.
    }

    void ensurePermission().then(async () => {
      // Bootstrap: subscribe to whatever's live right now.
      try {
        const initial = await api.sessionLiveSnapshot();
        await subscribeNew(initial);
      } catch {
        /* no-tauri env */
      }
      // Track session-list changes via the existing aggregate
      // channel. Payload: `LiveSessionSummary[]`.
      try {
        aliveUnlisten = await listen<LiveSessionSummary[]>(
          "live-all",
          (ev) => {
            void subscribeNew(ev.payload ?? []);
          },
        );
      } catch {
        /* ignore */
      }
    });

    return () => {
      if (aliveUnlisten) aliveUnlisten();
      for (const [, u] of subscriptions.current) {
        try {
          u();
        } catch {
          /* ignore */
        }
      }
      subscriptions.current.clear();
      recent.current.clear();
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
