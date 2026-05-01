// Activity preferences + live session feed + activity cards + trends.
// Sharded from src/api.ts; src/api/index.ts merges every
// domain slice into the canonical `api` object.

import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import type {
  ActivityCard,
  ActivityTrends,
  CardNavigate,
  CardsCount,
  CardsRecentQuery,
  CardsReindexResult,
  LiveSessionSummary,
  Preferences,
} from "../types";

/**
 * Frontend broadcast emitted after any preference mutation. The
 * payload IS the freshly-saved Preferences snapshot, so listeners
 * (useActivityNotifications, useCardNotifications) can update
 * directly from the event without round-tripping a second
 * `preferencesGet()`. The bare-emit form raced under network
 * jitter — two rapid setters could enqueue two reads, and an
 * older `preferencesGet()` resolving last would stomp the newer
 * value. Tauri events deliver in emission order, so passing the
 * snapshot through eliminates the ordering race entirely.
 *
 * Fire-and-forget — listen() subscribers already swallow non-Tauri
 * failures.
 */
const broadcastPrefsChanged = (prefs: Preferences) => {
  void emit("cp-prefs-changed", prefs).catch(() => {});
};

export const activityApi = {
  // ─── session_live (Activity feature) ─────────────────────────────

  /**
   * Start the live runtime (poll ~/.claude/sessions + tail transcripts).
   * Idempotent: repeated calls after a first successful start are
   * no-ops. The backend emits aggregate updates on the `live-all`
   * event channel and per-session deltas on `live::<sessionId>`.
   */
  /**
   * Partial update of the `activity_*` preference block. Any field
   * left undefined is preserved; the returned value is the refreshed
   * snapshot so the UI can round-trip without a separate GET.
   */
  preferencesSetActivity: (patch: {
    enabled?: boolean;
    consentSeen?: boolean;
    hideThinking?: boolean;
    excludedPaths?: string[];
  }) =>
    invoke<Preferences>("preferences_set_activity", {
      enabled: patch.enabled,
      consentSeen: patch.consentSeen,
      hideThinking: patch.hideThinking,
      excludedPaths: patch.excludedPaths,
    }).then((p) => {
      broadcastPrefsChanged(p);
      return p;
    }),

  /** Partial update of the `notify_*` preference block. */
  preferencesSetNotifications: (patch: {
    onError?: boolean;
    onIdleDone?: boolean;
    onStuckMinutes?: number | null;
    onOpDone?: boolean;
    onWaiting?: boolean;
    onUsageThresholds?: number[];
    onSubWindows?: boolean;
  }) =>
    invoke<Preferences>("preferences_set_notifications", {
      onError: patch.onError,
      onIdleDone: patch.onIdleDone,
      onStuckMinutes: patch.onStuckMinutes,
      onOpDone: patch.onOpDone,
      onWaiting: patch.onWaiting,
      onUsageThresholds: patch.onUsageThresholds,
      onSubWindows: patch.onSubWindows,
    }).then((p) => {
      broadcastPrefsChanged(p);
      return p;
    }),

  sessionLiveStart: () => invoke<void>("session_live_start"),

  /** Stop the live runtime. Drops all detail subscribers. */
  sessionLiveStop: () => invoke<void>("session_live_stop"),

  /**
   * Synchronous snapshot of currently-live sessions. Used by
   * `useSessionLive` on first mount (before the first `live-all`
   * event arrives) and as the resync answer after a gap.
   */
  sessionLiveSnapshot: () =>
    invoke<LiveSessionSummary[]>("session_live_snapshot"),

  /**
   * One-session snapshot for resync after `resync_required`.
   * Returns `null` when the session is no longer live.
   */
  sessionLiveSessionSnapshot: (sessionId: string) =>
    invoke<LiveSessionSummary | null>("session_live_session_snapshot", {
      sessionId,
    }),

  /**
   * Subscribe to per-session detail deltas. Backend forwards every
   * delta as a `live::<sessionId>` Tauri event; the caller listens
   * via `useTauriEvent` or raw `listen`.
   *
   * Single-subscriber per session — concurrent calls for the same
   * id will reject with `AlreadySubscribed`. Detach by calling
   * `sessionLiveUnsubscribe(sessionId)` (paired below); dropping the
   * frontend listener alone is NOT sufficient because the backend
   * forwarder keeps running until either the session ends or the
   * paired unsubscribe lands.
   */
  sessionLiveSubscribe: (sessionId: string) =>
    invoke<void>("session_live_subscribe", { sessionId }),

  /** Paired unsubscribe. Frontend listeners MUST call this before
   *  dropping their Tauri event listener — otherwise the backend
   *  task keeps forwarding until the session itself ends, and a
   *  re-subscribe on remount fails with AlreadySubscribed. */
  sessionLiveUnsubscribe: (sessionId: string) =>
    invoke<void>("session_live_unsubscribe", { sessionId }),

  /** Query the durable activity metrics store for the Trends view.
   *  Returns bucketed active-session counts + an error total for the
   *  requested window. Safe to call with `bucketCount: 0` → empty
   *  series. Unavailable metrics store → all-zero series, not
   *  an error. */
  activityTrends: (
    fromMs: number,
    toMs: number,
    bucketCount: number,
  ) =>
    invoke<ActivityTrends>("activity_trends", {
      fromMs,
      toMs,
      bucketCount,
    }),

  // Activity cards — per-event forensic surface (separate from the
  // live-strip activityTrends above; cards diagnose anomalies +
  // milestones from session JSONLs). See dev-docs/activity-cards-design.md.
  cardsRecent: (query: CardsRecentQuery) =>
    invoke<ActivityCard[]>("cards_recent", { query }),
  cardsCountNewSince: (query: CardsRecentQuery) =>
    invoke<CardsCount>("cards_count_new_since", { query }),
  cardsSetLastSeen: (cardId: number) =>
    invoke<void>("cards_set_last_seen", { cardId }),
  cardsNavigate: (cardId: number) =>
    invoke<CardNavigate | null>("cards_navigate", { cardId }),
  cardsBody: (cardId: number) =>
    invoke<string | null>("cards_body", { cardId }),
  cardsReindex: () => invoke<CardsReindexResult>("cards_reindex"),

};
