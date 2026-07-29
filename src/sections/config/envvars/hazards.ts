// The hazard taxonomy, in one place.
//
// Two surfaces need it — the inline warning on the row and the
// confirmation before a write lands — and they used to carry separate
// copies. That is a drift risk with teeth: renaming or adding a hazard
// would have updated the warning a user reads while leaving the
// confirmation they actually click describing something else.

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

/** Inline warning copy — what the row says while you are looking at it. */
export const HAZARD_WARNING: Record<NamedHazard, string> = {
  // Covers both endpoint overrides and NO_PROXY, which is a bypass list
  // rather than a destination — "an endpoint you control" was wrong for it.
  redirect:
    "Changes where Claude Code sends traffic, or which traffic bypasses your proxy. Only use destinations and bypass rules you trust.",
  trust_cert:
    "Changes which TLS certificates Claude Code trusts. A wrong value makes interception undetectable.",
  // The bucket is shell paths, a wrapper prefix, and CLAUDE_ENV_FILE: what
  // runs is the code this points AT, not the string itself.
  execute_code:
    "Names a binary, wrapper, or script that Claude Code runs. Only point it at local code you trust.",
  switch_project:
    "Switches which account or project your requests bill and log to.",
  disable_updates:
    "Stops Claude Code from updating itself, including security fixes.",
};

/** Confirmation copy — shorter, because it sits next to an Apply button
 *  the user is about to press. Same taxonomy, same source of truth. */
export const HAZARD_CONFIRM: Record<NamedHazard, string> = {
  redirect: "This redirects Claude Code's traffic.",
  trust_cert: "This changes which TLS certificates are trusted.",
  execute_code: "This value ends up in a command Claude Code runs.",
  switch_project: "This changes which account or project you bill and log to.",
  disable_updates: "This stops Claude Code from updating itself.",
};

/** The plaintext-storage warning every secret write carries. */
export const SECRET_CONFIRM =
  "This writes the value into Claude Code's user settings file in plaintext. Claude Code does not encrypt it, and anything that can read the file can read the value.";

/** Build the one combined confirmation body for a write. Secret and
 *  hazardous rows get a single dialog, never two stacked ones. */
export function confirmBody(secret: boolean, hazards: Hazard[]): string {
  const parts: string[] = [];
  if (secret) parts.push(SECRET_CONFIRM);
  for (const h of namedHazards(hazards)) parts.push(HAZARD_CONFIRM[h]);
  return parts.join(" ");
}
