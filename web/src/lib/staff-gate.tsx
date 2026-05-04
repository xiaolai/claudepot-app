import type { ReactNode } from "react";
import Link from "next/link";

import { auth } from "@/lib/auth";
import {
  getCurrentUser,
  isStaff as isStaffFixture,
} from "@/lib/auth-shim";

/**
 * Server-side staff gate for /admin/* routes.
 *
 * Resolves the viewer in this order:
 *   1. Real Auth.js session — staff iff `session.user.role` is
 *      "staff" or "system" (the canonical DB roles).
 *   2. Dev `?as=` shim — staff iff the fixture user passes
 *      isStaff() (DEV_STAFF_USERNAMES + is_system flag). The shim
 *      itself is null in production by contract — see getCurrentUser
 *      in prototype-fixtures.ts.
 *
 * Returns a JSX gate to render directly when access is refused, or
 * `null` to mean "let the page through." Use:
 *
 *   const gate = await staffGate(sp);
 *   if (gate) return gate;
 *   // ... staff-only content here
 *
 * Two refusal modes get different copy:
 *   - anonymous: no real session, no shim user → offer Sign in
 *   - not-staff: signed in / shim active but not a staff role → tell
 *     them the door is closed; no link to elsewhere because they
 *     have nowhere useful to go inside /admin
 */
export async function staffGate(searchParams: {
  as?: string;
}): Promise<ReactNode | null> {
  const session = await auth();

  if (session?.user) {
    const role = session.user.role;
    if (role === "staff" || role === "system") return null;
    return <NotStaffGate username={session.user.username} />;
  }

  const devUser = getCurrentUser(searchParams);
  if (devUser) {
    if (isStaffFixture(devUser)) return null;
    return <NotStaffGate username={devUser.username} />;
  }

  return <AnonymousGate />;
}

function AnonymousGate() {
  return (
    <div className="proto-admin-gate">
      <p className="proto-empty proto-empty-spaced">
        Staff only. <Link href="/login">Sign in</Link> to continue.
      </p>
    </div>
  );
}

function NotStaffGate({ username }: { username: string }) {
  return (
    <div className="proto-admin-gate">
      <p className="proto-empty proto-empty-spaced">
        Staff only. You&rsquo;re signed in as @{username}, but this area is
        restricted to moderators.
      </p>
    </div>
  );
}
