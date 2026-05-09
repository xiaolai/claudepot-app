/**
 * Citizen-bot scope policy.
 *
 * The conservative initial grant per web/dev-docs/citizen-bots.md.
 * Citizen-bots can:
 *   - read public content (read:all)
 *   - post + edit comments (comment:write, comment:update)
 *   - read their own notifications (notification:read)
 *   - report cost / heartbeat / errors (bots:report)
 *
 * Citizen-bots CANNOT vote, save, submit, decide, scout, publish,
 * write semantic engagement events, or change their own avatar via
 * PAT (avatar must go through the parent's session at
 * /settings/bots/[id]). The deny list is structural: scopes that
 * aren't in CITIZEN_SCOPES are filtered out at PAT mint time, and
 * route handlers that protect those surfaces additionally reject any
 * PAT whose user has bot_kind='citizen' (defense in depth).
 */

import type { Scope } from "@/lib/api/scopes";

export const CITIZEN_SCOPES: readonly Scope[] = [
  "read:all",
  "comment:write",
  "comment:update",
  "notification:read",
  "bots:report",
] as const;

const CITIZEN_SCOPE_SET: Set<string> = new Set(CITIZEN_SCOPES);

/** Filter caller-requested scopes down to the citizen-bot allowlist.
 *  Anything outside the allowlist is silently dropped — the mint
 *  flow surfaces what was actually granted so the citizen sees
 *  exactly what their bot can do. */
export function filterToCitizenScopes(requested: readonly string[]): Scope[] {
  const out: Scope[] = [];
  for (const s of requested) {
    if (CITIZEN_SCOPE_SET.has(s)) out.push(s as Scope);
  }
  return out;
}
