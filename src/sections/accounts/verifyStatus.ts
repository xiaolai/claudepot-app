import type { AccountSummary, VerifyStatus } from "../../types";

/**
 * The one place the accounts UI decides what a `verify_status` *means*.
 *
 * Four surfaces used to answer that question independently — the health
 * chips' `categorize`, the banner's `isAnomaly`, the banner's
 * `anomalyCopy`, and the footer's label ternary. Adding `signed_out`
 * meant editing all four, and missing any one of them produced a
 * plausible-looking half-state: a terminal account counted as healthy,
 * or one with no banner to act on. That is the same defect shape as the
 * bug the status was added for, so it does not get to live in the UI
 * layer either.
 *
 * Mirrors `claudepot_core::account::VerifyOutcome`'s own split
 * (`status_is_terminal` / `status_remedy`) rather than inventing a
 * second taxonomy.
 */
export type VerifyKind =
  /** Confirmed good. */
  | "ok"
  /** Not yet checked, or the check could not complete. Nothing is
   *  known to be wrong, and nothing is for the user to do. */
  | "unknown"
  /** The slot authenticates as someone else. */
  | "drift"
  /** Terminal — only a re-login recovers it. */
  | "needsLogin";

export function verifyKind(status: VerifyStatus): VerifyKind {
  switch (status) {
    case "ok":
      return "ok";
    case "drift":
      return "drift";
    case "rejected":
    case "signed_out":
      return "needsLogin";
    case "never":
    case "network_error":
      return "unknown";
    default:
      // Exhaustive: adding a member to `VerifyStatus` without a case
      // here is a compile error, not a silent fall-through to a
      // reassuring default. `never` here proves the switch covers the
      // union; the runtime arm only fires if a backend sends a status
      // this build has never heard of, and it resolves to "unknown"
      // (keep checking) rather than "ok" (claim health we cannot
      // vouch for).
      return assertNeverStatus(status);
  }
}

function assertNeverStatus(status: never): VerifyKind {
  void status;
  return "unknown";
}

/**
 * True when no retry clears this state — the user has to act.
 *
 * `drift` is excluded on purpose: it needs attention, but `verify`
 * can clear it, so it is not a re-login. `requiresAttention` is the
 * predicate for "show a banner"; this one is for "the only way out is
 * signing in again".
 */
export function requiresRelogin(status: VerifyStatus): boolean {
  return verifyKind(status) === "needsLogin";
}

/**
 * True when the account needs a human, whatever the remedy.
 *
 * Takes the whole summary rather than just the status because an
 * unreadable credential blob (`credentials_healthy === false`) is an
 * attention state that no `verify_status` records.
 */
export function requiresAttention(a: AccountSummary): boolean {
  if (!a.credentials_healthy) return true;
  const kind = verifyKind(a.verify_status);
  return kind === "drift" || kind === "needsLogin";
}
