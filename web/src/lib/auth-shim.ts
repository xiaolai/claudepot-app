/**
 * DEV-ONLY auth shim — `?as=<username>` simulation that coexists with
 * real Auth.js sessions in dev. Removed entirely in phase 3 (see the
 * header comment in src/lib/auth.ts).
 */

import { getUser, type User } from "@/lib/prototype-fixtures";

/**
 * Returns the fixture user for `?as=<username>` in non-production
 * environments. Always returns null in production, regardless of
 * input — this is the load-bearing privacy guard. Without it, any
 * anonymous prod visitor could append `?as=ada` to a personal-hub or
 * admin route and impersonate that user's view (read their saved
 * bookmarks, upvotes, pending submissions, notifications, or — for
 * staff handles — see the moderation queue). Callers therefore do
 * NOT need to wrap the call in
 * `process.env.NODE_ENV === "production" ? null : ...`; the gate
 * lives here so it cannot be forgotten at any one site.
 */
export function getCurrentUser(searchParams: {
  as?: string | string[];
}): User | null {
  if (process.env.NODE_ENV === "production") return null;
  const raw = searchParams.as;
  const username = Array.isArray(raw) ? raw[0] : raw;
  if (!username) return null;
  return getUser(username) ?? null;
}

const DEV_STAFF_USERNAMES = new Set(["ada", "lixiaolai"]);

export function isStaff(user: User | null): boolean {
  if (!user) return false;
  if (user.is_system) return true;
  return DEV_STAFF_USERNAMES.has(user.username);
}

/** Compose a URL preserving the ?as= query param if present. */
export function withAuth(href: string, currentUser: User | null): string {
  if (!currentUser) return href;
  const sep = href.includes("?") ? "&" : "?";
  return `${href}${sep}as=${currentUser.username}`;
}
