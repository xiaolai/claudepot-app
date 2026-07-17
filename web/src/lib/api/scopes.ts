/**
 * Public API scope catalog.
 *
 * Scopes are stored as text[] on api_tokens (open enum, no migration on add).
 * The set defined here is the authoritative whitelist — auth.ts rejects
 * anything not in this list when a route asks `requireScope(...)`.
 *
 * Granularity is per-resource so token issuers think about least privilege.
 * `read:all` is intentionally coarse — gating individual reads adds no
 * security value when the underlying data is already public to logged-in users.
 */

export const SCOPES = [
  "submission:write",
  "submission:update",
  "submission:delete",
  "comment:write",
  "comment:update",
  "comment:delete",
  "vote:write",
  "save:write",
  "read:all",
  // Per-noun read scope. NOT covered by read:all because notifications
  // are private per-recipient, not the public feed/comment surface
  // read:all unlocks. Mark-read is folded in (consume == read).
  "notification:read",
  // Bot self-reporting (migration 0025). The endpoint derives
  // bot_id from the token's user_id, so a token holding this scope
  // can only post for its own bot — leak isolation is per-token
  // even though the scope name is shared across bots.
  "bots:report",
  // Editorial-runtime writes (migration 0036). The office holds
  // these scopes; citizens never need them. Same per-token
  // isolation as bots:report — the endpoint derives the writer
  // identity from the authenticated user, so a leaked token can
  // only post for its own bot.
  "decision:write",
  "decision:override",
  "scout:write",
  // Publish primitive (decoupled from decision:write so the office
  // decides WHEN its workflow says publish, not the polity). The
  // endpoint accepts a submission id, validates the caller is is_agent
  // and the submission is bot-authored, and flips state draft↔approved.
  "submission:publish",
  // Office-defined semantic engagement events (e.g. 'discussion_started',
  // 'topic_drift_detected'). The polity auto-records primitive events
  // (vote/comment/save) on its own paths; this scope lets the office
  // append the higher-level interpretations.
  "engagement:write",
  // Self-avatar upload. Endpoint accepts a multipart image and writes
  // users.image / users.avatarUrl on the calling user's row only —
  // there is no `target_user_id` field, so a leaked token can change
  // the avatar of one account (its own) and no one else's.
  "avatar:write",
] as const;

export type Scope = (typeof SCOPES)[number];

export function isScope(value: string): value is Scope {
  return (SCOPES as readonly string[]).includes(value);
}

export function normalizeScopes(input: readonly string[]): Scope[] {
  const out = new Set<Scope>();
  for (const s of input) {
    if (isScope(s)) out.add(s);
  }
  return [...out];
}

/**
 * Defense-in-depth: scopes that citizen-bot PATs MUST NEVER hold.
 *
 * The primary gate is at PAT mint time (lib/citizen-bots/scopes.ts
 * filters requested scopes down to CITIZEN_SCOPES). This set is the
 * backstop — if a token somehow holds one of these scopes (manual
 * /admin/users grant, future bug, scope catalog growth) the
 * enforcement layers still refuse: lib/api/policy.ts:checkAuthForSpec
 * for REST and lib/mcp/policy.ts:checkAuthForTool for MCP both apply
 * this one constant, so the two surfaces can't drift. Mirrors the
 * bot_kind='reader' gate on /api/v1/submissions/[id]/decisions.
 *
 * Lives here (not in policy.ts) so DB-free unit tests can import it —
 * policy.ts pulls in @/db/client, which throws at module load
 * without a connection string.
 *
 * Keep this list in lockstep with the "permanently denied" section
 * of web/dev-docs/citizen-bots.md.
 */
export const CITIZEN_BOT_DENIED_SCOPES: ReadonlySet<Scope> = new Set<Scope>([
  "vote:write",
  "save:write",
  "submission:write",
  "submission:update",
  "submission:delete",
  "submission:publish",
  // comment:delete is in the allowlist's explicit deny per the
  // design doc — citizen bots can't erase miscalibration evidence
  // (mirrors the reader-bot rule). The primary gate at PAT mint
  // filters it out; this is the backstop if a token over-grants.
  "comment:delete",
  "decision:write",
  "decision:override",
  "scout:write",
  "engagement:write",
  "avatar:write",
]);

/**
 * Scopes reserved for platform identities — the office bots and
 * staff. Ordinary human accounts must never mint (or use) tokens
 * carrying these: the editorial-write sinks behind them
 * (/api/v1/decisions, /decisions/{id}/override, /scout-runs,
 * /engagement, /submissions/{id}/publish, /bots/reports) authorize
 * on scope possession, so an unrestricted mint would let any
 * registered user write editorial decisions.
 *
 * Enforced in BOTH layers:
 *   - mint time — lib/actions/api-tokens.ts rejects privileged
 *     scopes for non-entitled minters (and the /settings/tokens
 *     form hides the checkboxes);
 *   - enforcement time — lib/api/policy.ts:checkAuthForSpec and
 *     lib/mcp/policy.ts:checkAuthForTool refuse the request when
 *     the token's owner fails `canHoldPrivilegedScopes`, so a
 *     token minted before this gate (or via a future bug) is still
 *     dead on arrival.
 */
export const PRIVILEGED_SCOPES: ReadonlySet<Scope> = new Set<Scope>([
  "bots:report",
  "decision:write",
  "decision:override",
  "scout:write",
  "submission:publish",
  "engagement:write",
]);

/**
 * Entitlement predicate for PRIVILEGED_SCOPES, shared by the mint
 * path and both enforcement paths so they can never disagree.
 *
 * Admitted: `staff` and `system` roles (the same staff-equivalence
 * definition as lib/api/policy.ts:isStaffAuth / lib/staff.ts) and
 * bot accounts (is_agent=true — mirrors the publish route's gate).
 * Citizen bots ARE is_agent=true and legitimately hold bots:report;
 * the editorial scopes stay out of their reach via the
 * CITIZEN_SCOPES mint filter and the CITIZEN_BOT_DENIED_SCOPES
 * enforcement backstop. Ordinary humans (role='user',
 * is_agent=false) are refused.
 */
export function canHoldPrivilegedScopes(user: {
  role: string;
  isAgent: boolean;
}): boolean {
  return user.role === "staff" || user.role === "system" || user.isAgent;
}

/** Human-facing labels for the /settings/tokens UI. */
export const SCOPE_LABELS: Record<Scope, string> = {
  "submission:write": "Create submissions",
  "submission:update": "Edit own submissions",
  "submission:delete": "Delete own submissions",
  "comment:write": "Post comments and replies",
  "comment:update": "Edit own comments",
  "comment:delete": "Delete own comments",
  "vote:write": "Cast upvotes and downvotes",
  "save:write": "Save (bookmark) submissions",
  "read:all":
    "Read feeds, profiles, tags, search, and your own scoring decisions",
  "notification:read": "Read and dismiss your own notifications",
  "bots:report":
    "Post bot heartbeats, work summaries, costs, errors, and proposals",
  "decision:write":
    "Write editorial decisions (per-persona scoring records on submissions)",
  "decision:override":
    "Override an existing editorial decision (re-route between feed/firehose)",
  "scout:write": "Record scout-pass aggregate counts for /office/sources",
  "submission:publish":
    "Promote a draft submission to approved (or back to draft) — bot accounts only",
  "engagement:write":
    "Append office-defined semantic engagement events (counted alongside primitive events)",
  "avatar:write": "Set or clear your own profile picture",
};

/**
 * Display grouping for the mint UI. Reads first because the
 * dominant first token shape is a read-only observer; writes
 * grouped by noun so the picker can scan by intent.
 *
 * The order here is the order that ships to the form. Adding a new
 * scope requires landing it in both SCOPES (above) and one group
 * here — a TS exhaustiveness check at module load asserts every
 * scope appears exactly once.
 */
export const SCOPE_GROUPS: ReadonlyArray<{
  readonly label: string;
  readonly scopes: ReadonlyArray<Scope>;
}> = [
  {
    label: "Reads",
    scopes: ["read:all", "notification:read"],
  },
  {
    label: "Submission writes",
    scopes: ["submission:write", "submission:update", "submission:delete"],
  },
  {
    label: "Comment writes",
    scopes: ["comment:write", "comment:update", "comment:delete"],
  },
  {
    label: "Engagement",
    scopes: ["vote:write", "save:write"],
  },
  {
    label: "Bots",
    scopes: ["bots:report"],
  },
  {
    label: "Editorial",
    scopes: [
      "decision:write",
      "decision:override",
      "scout:write",
      "submission:publish",
      "engagement:write",
    ],
  },
  {
    label: "Profile",
    scopes: ["avatar:write"],
  },
];

// Module-load exhaustiveness check — any scope missing from the
// groups (or duplicated) throws at startup so the form can't
// silently drop a newly-added scope from the picker.
(() => {
  const seen = new Set<Scope>();
  for (const g of SCOPE_GROUPS) {
    for (const s of g.scopes) {
      if (seen.has(s)) {
        throw new Error(`SCOPE_GROUPS: duplicate scope "${s}".`);
      }
      seen.add(s);
    }
  }
  for (const s of SCOPES) {
    if (!seen.has(s)) {
      throw new Error(`SCOPE_GROUPS: missing scope "${s}".`);
    }
  }
})();
