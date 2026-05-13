// API surface for the `claude doctor` scrape pipeline. Distinct
// from `serviceStatusApi` (Claudepot's own health) — this carries
// CC's *own* doctor output. Backend caches for 60 s; pass
// `forceRefresh: true` to bypass.

import { invoke } from "@tauri-apps/api/core";

// "unknown" = we couldn't measure (parser failed AND no probe filled in
// fallback identity). Distinct from "healthy" so the UI renders grey,
// not green. The "Couldn't measure" vs "is sick" separation in
// HealthPane depends on this distinction.
export type DoctorSeverity = "unknown" | "healthy" | "warning" | "error";

export interface SectionEntry {
  text: string;
  treePrefix: string;
}

export interface DoctorSection {
  title: string;
  severity: DoctorSeverity;
  entries: SectionEntry[];
}

export type ParseStatus =
  | { kind: "ok" }
  | { kind: "degraded"; reason: string }
  | { kind: "failed"; reason: string };

export interface DoctorSnapshot {
  ccVersion: string | null;
  installType: string | null;
  installPath: string | null;
  severity: DoctorSeverity;
  sections: DoctorSection[];
  rawBytes: number;
  parseStatus: ParseStatus;
  capturedAtMs: number;
}

export const ccDoctorApi = {
  /**
   * Read the cached scrape or trigger a fresh one if the cache
   * is older than 60 s (or `forceRefresh` is true). Blocking call
   * on first invocation per minute — the pty scrape takes
   * 6–10 s end-to-end because of CC's npm dist-tag fetch in the
   * Updates section.
   *
   * Never throws on parse failure — a failed parse returns a
   * snapshot with `parseStatus.kind === "failed"` and severity
   * `warning`. The renderer should keep the previous snapshot
   * visible in that case rather than blanking the UI.
   */
  ccDoctorSnapshot: (forceRefresh?: boolean) =>
    invoke<DoctorSnapshot>("cc_doctor_snapshot", { forceRefresh }),

  /**
   * Reveal `~/.claudepot/doctor-parse-failures.jsonl` in the OS
   * file manager — or its parent directory if no parse failure
   * has been recorded yet. Surfaced from the Health pane only
   * when `parseStatus` is degraded/failed (and from the dev
   * console at any time).
   */
  ccDoctorOpenParseFailuresLog: () =>
    invoke<void>("cc_doctor_open_parse_failures_log"),
};
