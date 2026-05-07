/**
 * Resolves the policy-moderator persona's user id once per process.
 *
 * Ada is the moderator. Migration 0009_persona_bots already created
 * 'ada' as a system bot user (role='system', is_agent=true). We
 * reuse that row instead of a separate 'policy-moderator' account
 * so the audit trail attributes AI-driven moderation_log rows to
 * a real persona that readers can find via /office/persona/ada/.
 *
 * Migration 0018 still creates a 'policy-moderator' fallback user
 * for any environment that hasn't run 0009. It's kept around as a
 * defensive seed; this lookup prefers 'ada' and only falls back
 * to 'policy-moderator' if 'ada' isn't found (e.g. a fresh DB that
 * skipped the persona migration).
 */

import { inArray } from "drizzle-orm";
import { db } from "@/db/client";
import { users } from "@/db/schema";

const PRIMARY_USERNAME = "ada";
const FALLBACK_USERNAME = "policy-moderator";

let cached: string | null = null;
let pending: Promise<string> | null = null;

export async function getSystemUserId(): Promise<string> {
  if (cached) return cached;
  if (pending) return pending;
  pending = (async () => {
    // Prefer 'ada' (created by migration 0009_persona_bots). Fall
    // back to 'policy-moderator' (created by migration 0018) so a
    // freshly-bootstrapped DB that skipped the persona migration
    // still has a system actor for moderation_log rows.
    const candidates = await db
      .select({ id: users.id, username: users.username })
      .from(users)
      .where(inArray(users.username, [PRIMARY_USERNAME, FALLBACK_USERNAME]));
    const ada = candidates.find((u) => u.username === PRIMARY_USERNAME);
    const fallback = candidates.find(
      (u) => u.username === FALLBACK_USERNAME,
    );
    const u = ada ?? fallback;
    if (!u) {
      throw new Error(
        `Moderator persona user missing — run migration 0009_persona_bots.sql + 0018_policy_moderation.sql`,
      );
    }
    cached = u.id;
    return cached;
  })();
  try {
    return await pending;
  } finally {
    pending = null;
  }
}

/** Test-only override; do not call from production code. */
export function _setSystemUserIdForTesting(id: string | null): void {
  cached = id;
}
