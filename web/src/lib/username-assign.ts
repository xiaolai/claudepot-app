/**
 * DB-aware username assignment for first-time OAuth signups.
 *
 * Pulls candidates from `generateUsernameCandidates`, filters out
 * reserved names, and checks each against the `users` table until it
 * finds a free one. Caps attempts so a saturated namespace doesn't
 * spin forever — that's an operational signal, not something to
 * silently absorb.
 */

import { eq } from "drizzle-orm";

import type { DB } from "@/db/client";
import { users } from "@/db/schema";
import {
  generateUsernameCandidates,
  isReservedUsername,
  usernameFromEmail,
  usernameFromName,
} from "@/lib/username";

const MAX_ATTEMPTS = 200;

async function isFree(db: DB, candidate: string): Promise<boolean> {
  if (isReservedUsername(candidate)) return false;
  const hit = await db
    .select({ id: users.id })
    .from(users)
    .where(eq(users.username, candidate))
    .limit(1);
  return hit.length === 0;
}

/**
 * Assign a clean, unique username for a new OAuth user. Prefers a seed
 * derived from the OAuth display name; falls back to the email's local
 * part when name is missing or yields nothing usable. Throws if the
 * generator can't find a free candidate within MAX_ATTEMPTS — that
 * indicates namespace pressure that needs operator attention.
 */
export async function assignUsername(
  db: DB,
  input: { name: string | null | undefined; email: string },
): Promise<string> {
  const fromName = usernameFromName(input.name);
  const fromEmail = usernameFromEmail(input.email);
  // Prefer name-derived if it differs from the random fallback, else email.
  const seed = fromName.startsWith("user-") ? fromEmail : fromName;

  const gen = generateUsernameCandidates(seed);
  for (let i = 0; i < MAX_ATTEMPTS; i += 1) {
    const { value, done } = gen.next();
    if (done || !value) break;
    if (await isFree(db, value)) return value;
  }
  throw new Error(
    `Unable to allocate a unique username after ${MAX_ATTEMPTS} attempts (seed=${seed})`,
  );
}
