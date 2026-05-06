/**
 * Ban-ladder threshold constants. Lives in its own file (no DB
 * imports) so unit tests can read it without standing up a Postgres
 * connection.
 *
 * Strawman numbers per dev-docs/policy-moderator-plan.md §9; tune in
 * production. Constants are the only place to change them — silent
 * threshold drift between environments is worse than a conscious
 * code change, so no env vars yet.
 */

export const LADDER_THRESHOLDS = {
  /** Rejects (rolling 7d) needed to enter rung 3. */
  RUNG3_REJECT_TRIGGER: 3,
  RUNG3_WINDOW_DAYS: 7,
  /** Daily moderation-eligible-action cap once rung 3 is active. */
  RUNG3_DAILY_CAP: 5,

  /** Rejects (rolling 7d) needed to fire a ban-candidate flag. */
  RUNG4_REJECT_TRIGGER: 5,
  RUNG4_WINDOW_DAYS: 7,
} as const;

export const DAY_MS = 86_400_000;
