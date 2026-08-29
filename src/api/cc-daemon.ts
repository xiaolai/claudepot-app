// API surface for CC's background-worker count — how many workers the
// daemon is holding whose process is actually alive. It deliberately
// does NOT report supervisor liveness; see `claudepot-core::cc_daemon`
// for why that boolean could not be guarded and was removed rather
// than shipped unguarded. Powers the Sidebar Activity-strip bg-count
// badge (render-if-nonzero) and the rotation audit's "(N bg workers
// active)" suffix.
//
// Distinct from `ccDoctorApi` (CC's own self-diagnostic) — this is
// the supervisor that holds detached `/bg` sessions alive.
//
// The backend reads CC's `roster.json` and spawns nothing. It used to
// run `claude daemon status` on every poll, which on a Claude Code
// build predating that subcommand was billed as a headless model
// prompt — roughly 20K uncached input tokens a minute, rendering
// nothing (issue #94). No cache on the backend: the read is a small
// file plus a process-table probe, and the value changes with
// bg-session lifecycle, so a cached one would hide live transitions.

import { invoke } from "@tauri-apps/api/core";

export type DaemonParseStatus =
  | { kind: "ok" }
  | { kind: "degraded"; reason: string }
  | { kind: "failed"; reason: string };

export interface DaemonStatus {
  /**
   * Background workers whose process is **actually alive** — not the
   * roster's length. `null` means "couldn't tell" and must not be
   * rendered as a count; a healthy idle daemon reports `0`, not
   * `null`.
   *
   * Each entry is checked against its own recorded start time, so a
   * roster left behind by a dead daemon contributes nothing however
   * long it sits there, and a recycled PID is not mistaken for the
   * worker that used to own it. There is deliberately no `running`
   * flag beside this: CC's roster carries nothing that would let one
   * be guarded the same way, so the count is the whole signal.
   */
  bgWorkers: number | null;
  /** The roster file consulted — the first question when a count looks wrong. */
  rosterPath: string | null;
  parseStatus: DaemonParseStatus;
}

export const ccDaemonApi = {
  /**
   * One-shot read. Callers that poll should debounce on the renderer
   * side — 60s for the Sidebar badge is the default cadence.
   *
   * Never throws on a bad roster — a failed read returns a snapshot
   * with `parseStatus.kind === "failed"` and `bgWorkers: null`.
   * Consumers should treat that as "no signal" rather than "no
   * workers."
   */
  ccDaemonStatus: () => invoke<DaemonStatus>("cc_daemon_status"),
};
