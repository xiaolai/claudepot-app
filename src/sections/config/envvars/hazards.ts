// The hazard taxonomy, in one place.
//
// Two surfaces need it — the inline warning on the row and the
// confirmation before a write lands — and they used to carry separate
// copies. That is a drift risk with teeth: renaming or adding a hazard
// would have updated the warning a user reads while leaving the
// confirmation they actually click describing something else.

import { i18n } from "../../../lib/i18n";
import type { Hazard } from "../../../types/ccEnv";

/** A specific risk, named. `unknown` is deliberately absent: it is a
 *  statement about missing evidence, not a risk label, and inventing one
 *  would be the same sin as guessing a control type. */
export type NamedHazard = Exclude<Hazard, "unknown">;

/**
 * The hazards this build knows copy for.
 *
 * `hazards` arrives as JSON over IPC, so a hazard added to the Rust taxonomy
 * before this table is updated would otherwise pass the `!== "unknown"` test
 * and render `undefined` — silently dropping the safety copy on exactly the
 * row that needed it. Anything unrecognized is folded into the conservative
 * generic caution instead.
 */
const KNOWN: readonly NamedHazard[] = [
  "redirect",
  "trust_cert",
  "execute_code",
  "switch_project",
  "disable_updates",
];

export function namedHazards(hazards: Hazard[]): NamedHazard[] {
  return hazards.filter((h): h is NamedHazard =>
    (KNOWN as readonly string[]).includes(h),
  );
}

/** Whether a conservative generic caution is owed: either the backend said
 *  the risk is unestablished, or it named one this build has no copy for. */
export function hasUnnamedHazard(hazards: Hazard[]): boolean {
  return hazards.some(
    (h) => h === "unknown" || !(KNOWN as readonly string[]).includes(h),
  );
}

/** Inline warning copy — what the row says while you are looking at it.
 *  Resolved at call time, not at module load, so a language switch
 *  applies to a row already on screen. */
const HAZARD_WARNING_KEYS = {
  // Covers both endpoint overrides and NO_PROXY, which is a bypass list
  // rather than a destination — "an endpoint you control" was wrong for it.
  redirect: "envvars.hazardWarn.redirect",
  trust_cert: "envvars.hazardWarn.trustCert",
  // The bucket is shell paths, a wrapper prefix, and CLAUDE_ENV_FILE: what
  // runs is the code this points AT, not the string itself.
  execute_code: "envvars.hazardWarn.executeCode",
  switch_project: "envvars.hazardWarn.switchProject",
  disable_updates: "envvars.hazardWarn.disableUpdates",
} as const satisfies Record<NamedHazard, string>;

export function hazardWarning(h: NamedHazard): string {
  return i18n.t(HAZARD_WARNING_KEYS[h], { ns: "config" });
}

/** Confirmation copy — shorter, because it sits next to an Apply button
 *  the user is about to press. Same taxonomy, same source of truth. */
const HAZARD_CONFIRM_KEYS = {
  redirect: "envvars.hazardConfirm.redirect",
  trust_cert: "envvars.hazardConfirm.trustCert",
  execute_code: "envvars.hazardConfirm.executeCode",
  switch_project: "envvars.hazardConfirm.switchProject",
  disable_updates: "envvars.hazardConfirm.disableUpdates",
} as const satisfies Record<NamedHazard, string>;

/** The plaintext-storage warning every secret write carries. */
export function secretConfirm(): string {
  return i18n.t("envvars.secretConfirm", { ns: "config" });
}

/** Build the one combined confirmation body for a write. Secret and
 *  hazardous rows get a single dialog, never two stacked ones. */
export function confirmBody(secret: boolean, hazards: Hazard[]): string {
  const parts: string[] = [];
  if (secret) parts.push(secretConfirm());
  for (const h of namedHazards(hazards)) {
    parts.push(i18n.t(HAZARD_CONFIRM_KEYS[h], { ns: "config" }));
  }
  return parts.join(" ");
}
