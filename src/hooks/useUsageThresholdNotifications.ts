import { useEffect } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useEmit } from "../providers/AppStateProvider";

/**
 * `useUsageThresholdNotifications` — listens for the
 * `usage-threshold-crossed` event emitted by the Rust-side
 * `usage_watcher` task and routes each crossing through the
 * notification facade as `usageThreshold` (P1).
 *
 * The Rust side already enforces "fire once per (account × window
 * × threshold) per reset cycle" — the persisted `fired` set in
 * `usage_alert_state.json` is the source of truth. The dispatcher's
 * token bucket here gives a second-line defense in depth: if a bug
 * ever caused the Rust side to emit duplicates, the bucket would
 * absorb them. Different `dedupeKey` per threshold per window per
 * account so distinct legitimate alerts never compete for the same
 * bucket.
 *
 * Click target: the Accounts section, deep-linked to the email
 * whose usage crossed. The shell consumer translates this into
 * focusing Claudepot and switching to that section.
 *
 * Phase 3 migration: previously called `dispatchOsNotification`
 * directly. Now routes through `emit()` so user prefs (the
 * `usageThreshold` CategoryPrefs toggle) gate dispatch and the
 * bell records a single routed log row rather than the legacy
 * `source: "os"` shape.
 */
interface CrossingPayload {
  accountUuid: string;
  accountEmail: string | null;
  window: string;
  windowLabel: string;
  thresholdPct: number;
  utilizationPct: number;
  resetsAtIso: string | null;
  /**
   * Boot-race fix: Rust-side log entry id (the watcher pre-persists
   * the routed entry before emitting this event, so the bell shows
   * the threshold even when the renderer wasn't listening). We
   * forward as `preexistingLogId` so emit() doesn't write a second
   * entry. `null` only when the boot-fallback log state was
   * unreachable, in which case emit() writes one fresh entry.
   */
  logId: number | null;
}

export function useUsageThresholdNotifications(): void {
  const emit = useEmit();

  useEffect(() => {
    let active = true;
    let unlisten: UnlistenFn | null = null;

    void listen<CrossingPayload>("usage-threshold-crossed", (ev) => {
      if (!active || !ev.payload) return;
      const p = ev.payload;
      const title = `${p.accountEmail ?? "Account"} — ${p.windowLabel} at ${p.thresholdPct}%`;
      const body = formatBody(p.utilizationPct, p.resetsAtIso);
      void emit({
        category: "usageThreshold",
        title,
        body,
        // dedupeKey grain: account × window × threshold guarantees
        // two distinct legitimate crossings (e.g. 80% then 90% in
        // the same cycle) don't compete for the same bucket and
        // suppress each other.
        dedupeKey: `usage:${p.accountUuid}:${p.window}:${p.thresholdPct}`,
        target: p.accountEmail
          ? {
              kind: "app",
              route: { section: "accounts", email: p.accountEmail },
            }
          : { kind: "info" },
        // Forward the Rust-side log id when the watcher already
        // persisted the entry — boot-race fix for audit #4.
        preexistingLogId: p.logId ?? undefined,
      });
    })
      .then((fn) => {
        if (!active) fn();
        else unlisten = fn;
      })
      .catch(() => {
        /* non-tauri env */
      });

    return () => {
      active = false;
      if (unlisten) unlisten();
    };
  }, [emit]);
}

function formatBody(utilizationPct: number, resetsAtIso: string | null): string {
  const pct = `at ${utilizationPct.toFixed(1)}%`;
  if (!resetsAtIso) return pct;
  const ms = Date.parse(resetsAtIso);
  if (!Number.isFinite(ms)) return pct;
  const remaining = ms - Date.now();
  if (remaining <= 0) return `${pct} · resets now`;
  const minutes = Math.floor(remaining / 60_000);
  if (minutes < 60) return `${pct} · resets in ${minutes}m`;
  const hours = Math.floor(minutes / 60);
  const remMin = minutes % 60;
  if (hours < 24) {
    return remMin === 0
      ? `${pct} · resets in ${hours}h`
      : `${pct} · resets in ${hours}h ${remMin}m`;
  }
  const days = Math.floor(hours / 24);
  return `${pct} · resets in ${days}d`;
}
