/**
 * Resolves the policy-moderator system user id once per process.
 *
 * The username 'policy-moderator' is the stable lookup key — its
 * row is upserted in migration 0018, mirroring the
 * 0009_persona_bots pattern. We avoid hardcoding a UUID so dev /
 * preview / prod don't need to coordinate identifier values.
 *
 * The id is the actor on AI-driven moderation_log rows (action=
 * 'reject' with this id as staff_id). Staff actions still go
 * through the existing moderationAction code path.
 */

import { eq } from "drizzle-orm";
import { db } from "@/db/client";
import { users } from "@/db/schema";

const POLICY_MODERATOR_USERNAME = "policy-moderator";

let cached: string | null = null;
let pending: Promise<string> | null = null;

export async function getSystemUserId(): Promise<string> {
  if (cached) return cached;
  if (pending) return pending;
  pending = (async () => {
    const [u] = await db
      .select({ id: users.id })
      .from(users)
      .where(eq(users.username, POLICY_MODERATOR_USERNAME))
      .limit(1);
    if (!u) {
      throw new Error(
        `policy-moderator system user missing — run migration 0018_policy_moderation.sql`,
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
