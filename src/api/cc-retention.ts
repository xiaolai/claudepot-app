// Bindings for Settings → Retention. Shape mirrors
// `claudepot_core::cc_retention` (returned verbatim by the
// `retention_*` commands — no DTO).
//
// This covers CC's `cleanupPeriodDays`: the only Claude Code setting
// that destroys user data, and one CC's own UI never mentions.

import { invoke } from "@tauri-apps/api/core";

/**
 * Note there is no plain "off". `0` is not a low value on the same
 * scale as `30` — it means "write no transcripts and delete the
 * existing ones", so it gets its own mode and its own command.
 */
export type RetentionMode =
  /** Key absent — CC's 30-day default silently applies. */
  | "cc_default"
  /** An explicit positive day count. */
  | "explicit"
  /** `cleanupPeriodDays: 0` — persistence disabled entirely. */
  | "persistence_disabled"
  /** Negative value; invalidates CC's settings schema. */
  | "invalid";

export interface RetentionState {
  mode: RetentionMode;
  configured_days: number | null;
  /** What CC will actually enforce. For `invalid`, the conservative
   *  CC default rather than a value we'd be trusting to protect. */
  effective_days: number;
  is_cc_default: boolean;
  /** CC is skipping cleanup entirely because the settings file fails
   *  validation while explicitly containing the key. An invalid value
   *  therefore *protects* transcripts — so the fix is to correct the
   *  value, never to "restore the default", which would re-arm
   *  deletion. */
  cleanup_suppressed: boolean;
}

export interface TranscriptRisk {
  /** Top-level `projects/<slug>/*.jsonl|*.cast` — all CC will unlink. */
  total_transcripts: number;
  /** Past the cutoff already: deleted on CC's next launch. */
  already_deletable: number;
  /** Crosses the cutoff within `horizon_days` (excludes the above). */
  at_risk_within_horizon: number;
  oldest_ms: number | null;
  /** Files under session dirs that cleanup never walks. These are why
   *  the folder grows while history is destroyed. */
  nested_immortal: number;
  horizon_days: number;
  /** Part of the tree could not be read. The counts are a floor, not a
   *  total — never render reassurance while this is true. */
  scan_incomplete: boolean;
}

export interface RetentionReport {
  state: RetentionState;
  risk: TranscriptRisk;
  /** Raising the window buys time; it does not make the corpus
   *  durable. Always false until the archive contract ships. */
  is_durable_archive: boolean;
}

export const ccRetentionApi = {
  retentionReport: (horizonDays?: number) =>
    invoke<RetentionReport>("retention_report", {
      horizonDays: horizonDays ?? null,
    }),

  /** Positive day counts only — the backend rejects 0 and negatives. */
  retentionSet: (days: number) =>
    invoke<RetentionReport>("retention_set", { days }),

  /** Removes the key, re-arming CC's 30-day deletion. Confirm first. */
  retentionClear: () => invoke<RetentionReport>("retention_clear"),

  /** Writes 0: CC stops saving transcripts and deletes existing ones.
   *  Destructive; must sit behind a ConfirmDialog. */
  retentionDisablePersistence: () =>
    invoke<RetentionReport>("retention_disable_persistence"),
};
