import { eq } from "drizzle-orm";

import { auth } from "@/lib/auth";
import { db } from "@/db/client";
import { users } from "@/db/schema";

/**
 * Server-side staff verification for use inside server actions.
 *
 * Distinct from `staff-gate.tsx`'s `staffGate` (which renders JSX to
 * gate a page render): this returns the staff user's id-or-null so
 * an action can verify and immediately use the id when writing to
 * moderation_log. Reads `users.role` directly to avoid trusting the
 * session callback alone — defends against a stale session if the
 * role was downgraded since sign-in.
 */
export async function requireStaffId(): Promise<string | null> {
  const session = await auth();
  if (!session?.user?.id) return null;
  const [me] = await db
    .select({ role: users.role })
    .from(users)
    .where(eq(users.id, session.user.id))
    .limit(1);
  if (me?.role !== "staff" && me?.role !== "system") return null;
  return session.user.id;
}
