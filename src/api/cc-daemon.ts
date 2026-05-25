// API surface for `claude daemon status`. Returns the CC supervisor
// state — running, pid, uptime, background-worker count, sock dir,
// roster + log paths. Powers the Sidebar Activity-strip bg-count
// badge (render-if-nonzero) and the Activities dashboard tile.
//
// Distinct from `ccDoctorApi` (CC's own self-diagnostic) — this is
// the supervisor that holds detached `/bg` sessions alive. No cache
// on the backend; the scrape is ~50ms and the value changes with
// bg-session lifecycle, so cached values would hide live transitions
// for no visible win.

import { invoke } from "@tauri-apps/api/core";

export type DaemonParseStatus =
  | { kind: "ok" }
  | { kind: "degraded"; reason: string }
  | { kind: "failed"; reason: string };

export interface DaemonStatus {
  running: boolean;
  pid: number | null;
  uptimeSecs: number | null;
  /**
   * Active background workers in `roster.json` at scrape time.
   * `null` only when the parser couldn't pin the line down — a clean
   * idle daemon reports `0`, not `null`.
   */
  bgWorkers: number | null;
  sockDir: string | null;
  controlSock: string | null;
  rosterPath: string | null;
  logPath: string | null;
  parseStatus: DaemonParseStatus;
}

export const ccDaemonApi = {
  /**
   * One-shot scrape. Cheap (~50ms in the idle case). Callers that
   * poll should debounce on the renderer side — 60s for the Sidebar
   * badge is the default cadence.
   *
   * Never throws on parse failure — a failed parse returns a
   * snapshot with `parseStatus.kind === "failed"` and `bgWorkers: null`.
   * Consumers should treat a failed parse as "no signal" rather
   * than "no workers."
   */
  ccDaemonStatus: () => invoke<DaemonStatus>("cc_daemon_status"),
};
