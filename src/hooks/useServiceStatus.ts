import { useCallback, useEffect, useState } from "react";
import { api } from "../api";
import type {
  LatencyReport,
  ServiceStatusSummary,
  StatusTier,
} from "../api/service-status";
import { useTauriEvent } from "./useTauriEvent";

/** Worst-of summary across the page-status tier and the latency tier
 *  — what the StatusBar dot uses to pick a color. */
export function combinedTier(
  summary: StatusTier,
  latency: StatusTier,
): StatusTier {
  // Order of severity, ascending: ok < unknown < degraded < down.
  // Unknown ranks below degraded so a never-probed dot doesn't paint
  // amber when status-page polling is healthy.
  const rank: Record<StatusTier, number> = {
    ok: 0,
    unknown: 1,
    degraded: 2,
    down: 3,
  };
  return rank[summary] >= rank[latency] ? summary : latency;
}

interface UseServiceStatusOpts {
  /** When false, the hook stays inert — no polling, no event listener,
   *  no probe. Used to short-circuit when both feature toggles are
   *  disabled in Settings. */
  enabled: boolean;
  /** Mirrors `Preferences.service_status.poll_status_page`. When the
   *  user disables status-page polling, the cached summary tier stops
   *  contributing to `tier` — otherwise a previously-recorded
   *  Down/Degraded would stick around until the renderer reloads, even
   *  though the user just told us not to track this anymore. */
  pollStatusPage: boolean;
  /** When true, trigger a HEAD probe each time the window gains focus.
   *  See `dev-docs/network-status.md` for why this is on-focus rather
   *  than continuous. */
  probeOnFocus: boolean;
}

interface UseServiceStatusResult {
  summary: ServiceStatusSummary | null;
  latency: LatencyReport | null;
  /** True while a probe is in flight. Distinct from "no data yet" so
   *  the StatusBar dot can show a subtle pulse instead of staying
   *  grey. */
  probing: boolean;
  /** Combined worst-of tier for the dot color. */
  tier: StatusTier;
  /** Refresh the cached summary + latency from Rust. Cheap. */
  refresh: () => Promise<void>;
  /** Trigger a fresh probe. Hits the network (5s worst case). */
  probeNow: () => Promise<void>;
}

/**
 * Hook backing both the StatusBar dot and the Settings → Network
 * panel. Single source of truth so the two surfaces never disagree.
 *
 * Lifecycle:
 *   - mount: read cached summary + latency from Rust (fast, no network).
 *   - on `service-status::updated` event: re-read summary.
 *   - on window focus (if `probeOnFocus`): trigger a HEAD batch.
 *
 * Disabled mode (`enabled: false`) keeps state at the last-known
 * values so re-enabling doesn't blank the dot mid-render.
 */
export function useServiceStatus(
  opts: UseServiceStatusOpts,
): UseServiceStatusResult {
  const { enabled, pollStatusPage, probeOnFocus } = opts;

  const [summary, setSummary] = useState<ServiceStatusSummary | null>(null);
  const [latency, setLatency] = useState<LatencyReport | null>(null);
  const [probing, setProbing] = useState(false);

  const refresh = useCallback(async () => {
    if (!enabled) return;
    try {
      const [s, l] = await Promise.all([
        api.serviceStatusSummary(),
        api.serviceStatusLatency(),
      ]);
      setSummary(s);
      setLatency(l);
    } catch {
      // Best-effort. Keep last-known state; the user's view doesn't
      // need to react to a transient IPC blip.
    }
  }, [enabled]);

  const probeNow = useCallback(async () => {
    if (!enabled) return;
    setProbing(true);
    try {
      const r = await api.serviceStatusProbeNow();
      setLatency(r);
    } catch {
      // Same rationale as `refresh`. The next on-focus call will
      // retry; surfacing every transient probe failure as a toast
      // would be more noise than signal.
    } finally {
      setProbing(false);
    }
  }, [enabled]);

  // Initial load.
  useEffect(() => {
    if (!enabled) return;
    void refresh();
  }, [enabled, refresh]);

  // Background-watcher refresh signal.
  useTauriEvent<unknown>(
    enabled ? "service-status::updated" : null,
    () => {
      void refresh();
    },
  );

  // On-focus probe. Fires once on mount when `probeOnFocus` is true,
  // then on each subsequent `window` focus event. The browser focus
  // event is the same for the Tauri webview, so this works without a
  // Tauri-specific channel.
  useEffect(() => {
    if (!enabled || !probeOnFocus) return;
    void probeNow();
    const handler = () => {
      void probeNow();
    };
    window.addEventListener("focus", handler);
    return () => window.removeEventListener("focus", handler);
  }, [enabled, probeOnFocus, probeNow]);

  // When polling is off, suppress the cached summary tier so a
  // previously-Degraded/Down result doesn't stick around as a stale
  // signal after the user disabled the feature.
  const tier = combinedTier(
    pollStatusPage ? (summary?.tier ?? "unknown") : "unknown",
    latency?.tier ?? "unknown",
  );

  return { summary, latency, probing, tier, refresh, probeNow };
}
