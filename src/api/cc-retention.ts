// Bindings for Settings → Retention. Shape mirrors
// `claudepot_core::cc_retention` (returned verbatim by the
// `retention_*` commands — no DTO).
//
// This covers CC's `cleanupPeriodDays`: the only Claude Code setting
// that destroys user data, and one CC's own UI never mentions.

import { invoke } from "@tauri-apps/api/core";

/**
 * Note there is no plain "off", and no way to write one. `0` is not a
 * low value on the same scale as `30`: Claude Code's floor is `1`, and
 * a `0` on disk fails validation, which makes CC skip cleanup entirely.
 * Disabling transcript writes is not reachable from settings at all —
 * `--no-session-persistence` is `--print`-only and `persistSession:
 * false` is an SDK option — so there is no "stop saving" command here.
 */
export type RetentionMode =
  /** Key absent — CC's 30-day default silently applies. */
  | "cc_default"
  /** An explicit day count of 1 or more. */
  | "explicit"
  /**
   * `cleanupPeriodDays: 0` — written by an older Claudepot, when CC
   * still honoured it as "stop persisting". CC now rejects it, so it
   * does the opposite: transcripts are written and cleanup is
   * suppressed. Distinct from `invalid` because the repair copy differs
   * — this is a promise that broke upstream, not a typo.
   */
  | "legacy_zero"
  /** Below 1, or not an integer; invalidates CC's settings schema. */
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
  /** `desktopSessionCleanupPeriodDays` (CC 2.1.248+) holds a value CC's
   *  schema rejects, which suppresses cleanup for *every* transcript.
   *
   *  **The producer enforces** that this is only true when the settings
   *  file read cleanly and `cleanupPeriodDays` itself parsed — an
   *  unreadable file resolves *both* keys to invalid, and a flag set
   *  from the desktop key alone could not tell that apart from a
   *  genuinely bad desktop value. Consumers may therefore name the key
   *  without re-checking `mode`, and must: CC's own `/doctor` names it,
   *  and pointing the user at a valid `cleanupPeriodDays` would
   *  contradict it. */
  desktop_key_rejected: boolean;
}

export interface TranscriptRisk {
  /** Top-level `projects/<slug>/*.jsonl|*.cast` — all CC will unlink. */
  total_transcripts: number;
  /** Past the cutoff already: deleted on CC's next launch. */
  already_deletable: number;
  /** Crosses the cutoff within `horizon_days` (excludes the above). */
  at_risk_within_horizon: number;
  oldest_ms: number | null;
  /** Files under session dirs (`subagents/`, `workflows/`,
   *  `remote-agents/`, tool results).
   *
   *  **Deleted, not immortal** — verified against CC 2.1.250: unlinking
   *  a top-level `<uuid>.jsonl` also `rm -rf`s the sibling `<uuid>/`
   *  folder, with no per-file age check. Reported separately because
   *  none of these appear in `total_transcripts`, so that number alone
   *  understates the loss — not because they survive it. */
  nested_below_session: number;
  horizon_days: number;
  /** Part of the tree could not be read. The counts are a floor, not a
   *  total — never render reassurance while this is true. */
  scan_incomplete: boolean;
}

/** One directory CC ages out on the same `cleanupPeriodDays` timer,
 *  other than `projects/`. Counted in the unit CC deletes: files for
 *  some directories, immediate subdirectories for others. */
export interface SweptDir {
  rel: string;
  /** Prose for a reader — "file edit history", not "file-history". */
  what: string;
  kind: "content" | "cache";
  entries: number;
  already_deletable: number;
}

/** What `cleanupPeriodDays` is aging out beyond the transcript tree.
 *  Only `content` directories are counted; `cache_dirs_skipped` records
 *  how many were deliberately not, so the omission is stated rather
 *  than implied. */
export interface SweptElsewhere {
  dirs: SweptDir[];
  /** Counts are a floor. Never render them as a total while set. */
  scan_incomplete: boolean;
  cache_dirs_skipped: number;
}

export interface RetentionReport {
  state: RetentionState;
  risk: TranscriptRisk;
  /** Raising the window buys time; it does not make the corpus
   *  durable. Always false until the archive contract ships. */
  is_durable_archive: boolean;
  /** The rest of the same timer — see `SweptElsewhere`. */
  swept_elsewhere: SweptElsewhere;
}

export const ccRetentionApi = {
  retentionReport: (horizonDays?: number) =>
    invoke<RetentionReport>("retention_report", {
      horizonDays: horizonDays ?? null,
    }),

  /** Day counts of 1 or more — the backend rejects anything lower. */
  retentionSet: (days: number) =>
    invoke<RetentionReport>("retention_set", { days }),

  /** Removes the key, re-arming CC's 30-day deletion. Confirm first.
   *  One of two repairs for `legacy_zero` — `retentionSet(days >= 1)`
   *  also lifts the validation error, and is what the pane's presets
   *  use. This one is the restore-the-default path specifically. */
  retentionClear: () => invoke<RetentionReport>("retention_clear"),
};
