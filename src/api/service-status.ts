// Network-status feature. See `dev-docs/network-status.md`.
//
// Two distinct surfaces:
//   1. Status-page summary (cached in Rust by the background watcher).
//   2. Per-host latency (probed on demand via HEAD requests).
//
// Both come back through Tauri commands rather than browser `fetch`
// — see `.claude/rules/architecture.md` IPC trust + secret direction.
// The renderer never opens an outbound connection.

import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import type { Preferences } from "../types";

export type StatusTier = "ok" | "degraded" | "down" | "unknown";

export interface ComponentStatus {
  id: string;
  name: string;
  /** Raw Statuspage status (`operational | degraded_performance | …`). */
  status: string;
  tier: StatusTier;
}

export interface ServiceIncident {
  id: string;
  name: string;
  /** `investigating | identified | monitoring | resolved | postmortem`. */
  status: string;
  /** `none | minor | major | critical`. */
  impact: string;
  createdAt: string;
  shortlink: string | null;
}

export interface ServiceStatusSummary {
  tier: StatusTier;
  /** Page-level Statuspage indicator, null before first successful poll. */
  indicator: string | null;
  description: string | null;
  components: ComponentStatus[];
  incidents: ServiceIncident[];
  /** ms since epoch of last successful poll; null before first poll. */
  fetchedAtMs: number | null;
  /** Last poll error message; null on success or before first poll. */
  lastError: string | null;
}

export type LatencyKind = "ok" | "timeout" | "error";

export interface HostLatency {
  name: string;
  url: string;
  kind: LatencyKind;
  /** ms when `kind == "ok"`; null otherwise. */
  ms: number | null;
  /** Error message when `kind == "error"`; null otherwise. */
  message: string | null;
}

export interface LatencyReport {
  tier: StatusTier;
  probedAtMs: number;
  hosts: HostLatency[];
}

export const serviceStatusApi = {
  /** Read the cached status-page summary (refreshed by the Rust
   *  watcher). Pure read — never hits the network. */
  serviceStatusSummary(): Promise<ServiceStatusSummary> {
    return invoke<ServiceStatusSummary>("service_status_summary_get");
  },
  /** Read the most recent latency report. Returns
   *  `{ tier: "unknown", hosts: [] }` before any probe has run. */
  serviceStatusLatency(): Promise<LatencyReport> {
    return invoke<LatencyReport>("service_status_latency_get");
  },
  /** Trigger a fresh HEAD-probe batch. Worst-case wall time: 5s
   *  (the Rust-side per-host timeout). */
  serviceStatusProbeNow(): Promise<LatencyReport> {
    return invoke<LatencyReport>("service_status_probe_now");
  },

  /** Partial update of the `service_status` preference block.
   *  Emits `cp-prefs-changed` so the StatusBar dot picks up the new
   *  toggles without polling. */
  preferencesSetServiceStatus(patch: {
    pollStatusPage?: boolean;
    pollIntervalMinutes?: number;
    osNotifyOnStatusChange?: boolean;
    probeLatencyOnFocus?: boolean;
  }): Promise<Preferences> {
    return invoke<Preferences>("preferences_set_service_status", {
      pollStatusPage: patch.pollStatusPage,
      pollIntervalMinutes: patch.pollIntervalMinutes,
      osNotifyOnStatusChange: patch.osNotifyOnStatusChange,
      probeLatencyOnFocus: patch.probeLatencyOnFocus,
    }).then((p) => {
      // Same shape as activity.ts's `broadcastPrefsChanged` — payload
      // IS the new Preferences so listeners (the StatusBar dot) can
      // update without a second preferencesGet round-trip.
      void emit("cp-prefs-changed", p).catch(() => {});
      return p;
    });
  },
};

/** Map a tier to the CSS variable used to color the StatusBar dot.
 *  Defined here (not in the dot component) so the Settings panel
 *  uses the same mapping for its read-only indicator. */
export function tierColor(tier: StatusTier): string {
  switch (tier) {
    case "ok":
      return "var(--ok, var(--accent))";
    case "degraded":
      return "var(--warn)";
    case "down":
      return "var(--danger)";
    case "unknown":
      return "var(--fg-faint)";
  }
}

/** One-line human label for the dot tooltip / panel header. */
export function tierLabel(tier: StatusTier): string {
  switch (tier) {
    case "ok":
      return "All systems operational";
    case "degraded":
      return "Some services degraded";
    case "down":
      return "Services down";
    case "unknown":
      return "Status unknown";
  }
}
