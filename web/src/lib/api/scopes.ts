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
  "comment:write",
  "vote:write",
  "save:write",
  "read:all",
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

/** Human-facing labels for the /settings/tokens UI. */
export const SCOPE_LABELS: Record<Scope, string> = {
  "submission:write": "Create submissions",
  "comment:write": "Post comments and replies",
  "vote:write": "Cast upvotes and downvotes",
  "save:write": "Save (bookmark) submissions",
  "read:all": "Read feed, submissions, and comments",
};
