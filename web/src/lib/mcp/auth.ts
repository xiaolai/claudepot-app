/**
 * MCP authentication adapter.
 *
 * mcp-handler's withMcpAuth() expects a verifyToken(req, bearerToken)
 * callback that returns AuthInfo when the token is valid. We delegate
 * to the same findActiveTokenByPlaintext() the REST endpoints use, so
 * MCP and REST share one source of truth for token validity.
 *
 * The user/token IDs are stashed in AuthInfo.extra so tool handlers
 * can read them via extra.authInfo.extra.* without re-hitting the DB.
 */

import type { AuthInfo } from "@modelcontextprotocol/sdk/server/auth/types.js";
import { eq } from "drizzle-orm";

import { db } from "@/db/client";
import { users } from "@/db/schema";
import { findActiveTokenByPlaintext, markTokenUsed } from "@/lib/api/tokens";

export type ClaudepotAuthExtra = {
  userId: string;
  username: string;
  role: string;
  tokenId: string;
  tokenPrefix: string;
};

export async function verifyClaudepotToken(
  _req: Request,
  bearerToken?: string,
): Promise<AuthInfo | undefined> {
  if (!bearerToken) return undefined;

  const token = await findActiveTokenByPlaintext(bearerToken).catch(() => null);
  if (!token) return undefined;

  const [user] = await db
    .select({
      id: users.id,
      username: users.username,
      role: users.role,
    })
    .from(users)
    .where(eq(users.id, token.userId))
    .limit(1);

  if (!user || user.role === "locked") return undefined;

  // Best-effort last-used bump. MCP requests can fan out to many tool
  // calls per session — the bump still happens once per authenticate.
  await markTokenUsed(token.id).catch(() => {});

  const extra: ClaudepotAuthExtra = {
    userId: user.id,
    username: user.username,
    role: user.role,
    tokenId: token.id,
    tokenPrefix: token.displayPrefix,
  };

  return {
    // AuthInfo.token is intentionally redacted. The plaintext was already
    // verified upstream; tool handlers never need it. Keeping it out of
    // AuthInfo eliminates a leak surface (accidental serialization,
    // future tool that logs `extra.authInfo`, etc.).
    token: "redacted",
    clientId: user.id, // MCP convention: the principal id for the client
    scopes: token.scopes,
    expiresAt: token.expiresAt
      ? Math.floor(token.expiresAt.getTime() / 1000)
      : undefined,
    extra: extra as unknown as Record<string, unknown>,
  };
}
