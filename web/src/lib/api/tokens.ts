/**
 * Token generation, hashing, and lookup.
 *
 * Plaintext format: `shn_pat_<28 url-safe-base64 chars>` (168 bits of entropy).
 * Storage: SHA-256 hex digest of the full plaintext. The plaintext leaves
 * the server exactly once — at creation time — and is never logged.
 *
 * Lookup is a single indexed equality check on hashed_secret — O(log n)
 * dominated by DB latency, not literal constant-time. Side-channel
 * resistance against timing analysis is provided by the index; we do not
 * claim cryptographic constant-time semantics here.
 */

import { randomBytes, createHash } from "node:crypto";
import { eq } from "drizzle-orm";

import { db } from "@/db/client";
import { apiTokens } from "@/db/schema";

const TOKEN_PREFIX = "shn_pat_";
const RANDOM_BYTES = 21; // 21 bytes -> 28 base64url chars
const DISPLAY_PREFIX_LEN = 12; // "shn_pat_XXXX"

/**
 * Strict format validator: prefix + exactly 28 URL-safe-base64 chars.
 * Used by auth.ts to reject oversized / malformed bearer values BEFORE
 * paying for a SHA-256 hash and DB query.
 */
export const TOKEN_FORMAT_RE = /^shn_pat_[A-Za-z0-9_-]{28}$/;

export type ApiToken = typeof apiTokens.$inferSelect;

export function generateToken(): {
  plaintext: string;
  hashed: string;
  displayPrefix: string;
} {
  const random = randomBytes(RANDOM_BYTES).toString("base64url");
  const plaintext = `${TOKEN_PREFIX}${random}`;
  return {
    plaintext,
    hashed: hashToken(plaintext),
    displayPrefix: plaintext.slice(0, DISPLAY_PREFIX_LEN),
  };
}

export function hashToken(plaintext: string): string {
  return createHash("sha256").update(plaintext).digest("hex");
}

/**
 * Look up an active token by its plaintext. Returns null for any of:
 * malformed format, no row, revoked, expired.
 *
 * Active = revoked_at IS NULL AND (expires_at IS NULL OR expires_at > now())
 */
export async function findActiveTokenByPlaintext(
  plaintext: string,
): Promise<ApiToken | null> {
  if (!TOKEN_FORMAT_RE.test(plaintext)) return null;

  const hashed = hashToken(plaintext);
  const [row] = await db
    .select()
    .from(apiTokens)
    .where(eq(apiTokens.hashedSecret, hashed))
    .limit(1);

  if (!row) return null;
  if (row.revokedAt) return null;
  if (row.expiresAt && row.expiresAt.getTime() <= Date.now()) return null;
  return row;
}

export async function markTokenUsed(tokenId: string): Promise<void> {
  await db
    .update(apiTokens)
    .set({ lastUsedAt: new Date() })
    .where(eq(apiTokens.id, tokenId));
}
