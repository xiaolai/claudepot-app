/**
 * Citizen-bot lifecycle. Pure DB + token operations; no Tauri / Web UI
 * coupling. Called from server actions (lib/actions/citizen-bots.ts)
 * and unit-tested directly.
 *
 * Domain rules enforced here:
 *
 *   - parent must be a real human user (NOT an agent, NOT locked)
 *   - per-parent cap of CITIZEN_BOT_CAP_PER_PARENT live bots
 *   - usernames take the shape `<base>@bot` (suffix mandatory)
 *   - a citizen-bot's PAT scopes are filtered to CITIZEN_SCOPES at
 *     mint time
 *   - delete is soft: revoke all PATs, anonymize the row (mirror
 *     account deletion), drop FK so the parent can be deleted
 *     independently. Bot row stays for thread-attribution
 *     continuity.
 */

import { and, count, eq, gt, isNull, or, sql } from "drizzle-orm";

import { db } from "@/db/client";
import { apiTokens, apiTokenEvents, users } from "@/db/schema";
import { generateToken } from "@/lib/api/tokens";
import type { Scope } from "@/lib/api/scopes";

import {
  type CreateCitizenBotInput,
  type MintCitizenBotTokenInput,
  CITIZEN_BOT_CAP_PER_PARENT,
  composeCitizenBotUsername,
} from "./schemas";
import { CITIZEN_SCOPES, filterToCitizenScopes } from "./scopes";

const TOKEN_NAME_PREFIX = "citizen bot";

/** Per-bot active-token cap. Same shape as MAX_TOKENS_PER_USER on
 *  the human side (lib/actions/api-tokens.ts), tighter because
 *  citizen-bot tokens are a derived surface — three concurrent
 *  rotators cover any rotation strategy. */
const MAX_TOKENS_PER_BOT = 5;

export type OwnedBot = {
  id: string;
  username: string;
  displayName: string | null;
  bio: string | null;
  avatarUrl: string | null;
  image: string | null;
  createdAt: Date;
  tokenCount: number;
};

export type CreateCitizenBotResult =
  | { ok: true; bot: OwnedBot }
  | {
      ok: false;
      reason:
        | "cap_reached"
        | "parent_invalid"
        | "username_taken"
        | "username_collides_with_human";
      detail?: string;
    };

export async function createCitizenBot(
  parentUserId: string,
  input: CreateCitizenBotInput,
): Promise<CreateCitizenBotResult> {
  // Parent must exist, be a real human (NOT an agent), and not be locked.
  const [parent] = await db
    .select({
      id: users.id,
      isAgent: users.isAgent,
      role: users.role,
    })
    .from(users)
    .where(eq(users.id, parentUserId))
    .limit(1);
  if (!parent) {
    return { ok: false, reason: "parent_invalid", detail: "Parent user not found." };
  }
  if (parent.isAgent) {
    return {
      ok: false,
      reason: "parent_invalid",
      detail: "Bots cannot own bots. Use a human account.",
    };
  }
  if (parent.role === "locked") {
    return {
      ok: false,
      reason: "parent_invalid",
      detail: "Account is locked.",
    };
  }

  const username = composeCitizenBotUsername(input.baseUsername);

  // Atomic cap-and-insert. Two concurrent POSTs would otherwise
  // both pass the count check and exceed CITIZEN_BOT_CAP_PER_PARENT.
  // Same advisory-lock shape as the human-side mint flow in
  // lib/actions/api-tokens.ts — namespace 'citizen_bots:create:'
  // so we don't contend with the api_tokens namespace.
  let capDetail: string | null = null;
  let usernameTaken = false;
  let insertedRow: { id: string; createdAt: Date } | null = null;
  await db.transaction(async (tx) => {
    await tx.execute(
      sql`SELECT pg_advisory_xact_lock(hashtext(${"citizen_bots:create:" + parentUserId}))`,
    );

    const [{ n: liveCount = 0 } = { n: 0 }] = await tx
      .select({ n: count() })
      .from(users)
      .where(
        and(
          eq(users.ownerUserId, parentUserId),
          eq(users.botKind, "citizen"),
        ),
      );
    if (liveCount >= CITIZEN_BOT_CAP_PER_PARENT) {
      capDetail = `You already have ${liveCount} bots (cap: ${CITIZEN_BOT_CAP_PER_PARENT}).`;
      return;
    }

    // Insert — bot_kind='citizen' + owner_user_id flips the CHECK
    // constraint into "citizen ⇒ owner_user_id NOT NULL AND
    // is_agent = true." If we forgot any of those three, the
    // INSERT errors out at the DB layer.
    try {
      const [row] = await tx
        .insert(users)
        .values({
          username,
          name: input.displayName ?? input.baseUsername,
          email: `${username.replace("@", "+at+")}@bots.claudepot.local`,
          role: "user",
          isAgent: true,
          botKind: "citizen",
          ownerUserId: parentUserId,
          bio: input.bio ?? null,
        })
        .returning({ id: users.id, createdAt: users.createdAt });
      insertedRow = row;
    } catch (err) {
      // Username uniqueness is enforced by idx_users_username;
      // mapping the constraint violation surfaces a friendly
      // 422 instead of leaking the error message.
      const msg = err instanceof Error ? err.message : String(err);
      if (
        msg.includes("idx_users_username") ||
        msg.includes("idx_users_email")
      ) {
        usernameTaken = true;
        return;
      }
      throw err;
    }
  });

  if (capDetail !== null) {
    return { ok: false, reason: "cap_reached", detail: capDetail };
  }
  if (usernameTaken) {
    return {
      ok: false,
      reason: "username_taken",
      detail: `@${username} is already taken.`,
    };
  }
  // The closure-narrowing dance: TypeScript loses the assignment
  // narrowing across `await db.transaction(async ...)`, so the
  // bare `insertedRow !== null` check below stays as `never` to
  // TS even though the runtime knows it's set. The `as` is the
  // documented escape hatch, not a hidden bug — we just verified
  // both error branches above (capDetail / usernameTaken) have
  // returned, so by elimination the assignment must have run.
  const row = insertedRow as { id: string; createdAt: Date } | null;
  if (row === null) {
    return {
      ok: false,
      reason: "parent_invalid",
      detail: "Bot creation failed; please retry.",
    };
  }

  return {
    ok: true,
    bot: {
      id: row.id,
      username,
      displayName: input.displayName ?? input.baseUsername,
      bio: input.bio ?? null,
      avatarUrl: null,
      image: null,
      createdAt: row.createdAt,
      tokenCount: 0,
    },
  };
}

export async function listOwnedBots(parentUserId: string): Promise<OwnedBot[]> {
  const rows = await db
    .select({
      id: users.id,
      username: users.username,
      displayName: users.name,
      bio: users.bio,
      avatarUrl: users.avatarUrl,
      image: users.image,
      createdAt: users.createdAt,
      tokenCount: sql<number>`(
        SELECT COUNT(*)::int FROM ${apiTokens}
         WHERE ${apiTokens.userId} = ${users.id}
           AND ${apiTokens.revokedAt} IS NULL
      )`,
    })
    .from(users)
    .where(
      and(
        eq(users.ownerUserId, parentUserId),
        eq(users.botKind, "citizen"),
      ),
    )
    .orderBy(users.createdAt);
  return rows;
}

export type DeleteCitizenBotResult =
  | { ok: true }
  | { ok: false; reason: "not_found" | "not_owner" };

/**
 * Soft-delete: revoke all PATs, clear bio + avatar + display name,
 * keep the row for thread-attribution continuity, drop the FK so
 * the parent can be deleted independently. Mirrors the user-level
 * account-deletion shape in lib/actions/settings.ts.
 *
 * After delete the bot's username remains reserved (we don't free
 * it — that would let a different parent re-claim it and inherit
 * the comment history).
 */
export async function deleteCitizenBot(
  parentUserId: string,
  botId: string,
): Promise<DeleteCitizenBotResult> {
  const [bot] = await db
    .select({
      id: users.id,
      ownerUserId: users.ownerUserId,
      botKind: users.botKind,
    })
    .from(users)
    .where(eq(users.id, botId))
    .limit(1);
  if (!bot) return { ok: false, reason: "not_found" };
  if (bot.botKind !== "citizen") return { ok: false, reason: "not_owner" };
  if (bot.ownerUserId !== parentUserId) {
    return { ok: false, reason: "not_owner" };
  }

  await db.transaction(async (tx) => {
    // Revoke all live PATs.
    await tx
      .update(apiTokens)
      .set({ revokedAt: new Date() })
      .where(
        and(
          eq(apiTokens.userId, botId),
          isNull(apiTokens.revokedAt),
        ),
      );
    // Anonymize the bot row. We keep username (so threads stay
    // attributed) and is_agent=true (so the AI chip still renders),
    // but everything else clears. owner_user_id goes to NULL — the
    // bot becomes "orphaned" and is no longer counted toward the
    // parent's cap. The CHECK constraint flips on delete: a
    // bot_kind='citizen' row with owner_user_id=NULL is invalid.
    // So we also flip bot_kind to NULL — the row turns into a
    // generic deleted-agent placeholder.
    await tx
      .update(users)
      .set({
        ownerUserId: null,
        botKind: null,
        bio: null,
        avatarUrl: null,
        image: null,
        name: null,
        updatedAt: new Date(),
      })
      .where(eq(users.id, botId));
  });

  return { ok: true };
}

export type MintCitizenBotTokenResult =
  | {
      ok: true;
      plaintext: string;
      displayPrefix: string;
      grantedScopes: Scope[];
      droppedScopes: string[];
    }
  | {
      ok: false;
      reason:
        | "not_found"
        | "not_owner"
        | "no_valid_scopes"
        | "cap_reached"
        | "transient";
      detail?: string;
    };

export async function mintTokenForBot(
  parentUserId: string,
  botId: string,
  input: MintCitizenBotTokenInput,
): Promise<MintCitizenBotTokenResult> {
  const [bot] = await db
    .select({
      id: users.id,
      ownerUserId: users.ownerUserId,
      botKind: users.botKind,
    })
    .from(users)
    .where(eq(users.id, botId))
    .limit(1);
  if (!bot) return { ok: false, reason: "not_found" };
  if (bot.botKind !== "citizen") return { ok: false, reason: "not_owner" };
  if (bot.ownerUserId !== parentUserId) {
    return { ok: false, reason: "not_owner" };
  }

  // The user can request whatever; we filter to the allowlist.
  // If they asked for a scope we deny, we surface it in the
  // result so the UI can explain what was dropped.
  const requested = input.scopes.length > 0 ? input.scopes : [...CITIZEN_SCOPES];
  const granted = filterToCitizenScopes(requested);
  if (granted.length === 0) {
    return { ok: false, reason: "no_valid_scopes" };
  }
  const grantedSet = new Set<string>(granted);
  const dropped = requested.filter((s) => !grantedSet.has(s));

  const { plaintext, hashed, displayPrefix } = generateToken();

  // Hardened mint, mirroring the human-side path in
  // lib/actions/api-tokens.ts:
  //
  //   1. Per-bot advisory lock so concurrent mints serialize.
  //   2. Active-token cap re-checked inside the lock so two
  //      parallel mints can't both pass.
  //   3. apiTokenEvents 'mint' audit row in the same transaction —
  //      either both rows commit or neither does. A live token
  //      without an audit event is impossible.
  //
  // Re-check ownership inside the transaction too: the bot could
  // have been soft-deleted between our SELECT and the lock acquire.
  let capDetail: string | null = null;
  let ownerProblem: "not_found" | "not_owner" | null = null;
  let insertedTokenId: string | null = null;
  await db.transaction(async (tx) => {
    await tx.execute(
      sql`SELECT pg_advisory_xact_lock(hashtext(${"citizen_bots:mint:" + botId}))`,
    );

    const [recheck] = await tx
      .select({
        ownerUserId: users.ownerUserId,
        botKind: users.botKind,
      })
      .from(users)
      .where(eq(users.id, botId))
      .limit(1);
    if (!recheck) {
      ownerProblem = "not_found";
      return;
    }
    if (recheck.botKind !== "citizen" || recheck.ownerUserId !== parentUserId) {
      ownerProblem = "not_owner";
      return;
    }

    const [{ n: activeCount = 0 } = { n: 0 }] = await tx
      .select({ n: count() })
      .from(apiTokens)
      .where(
        and(
          eq(apiTokens.userId, botId),
          isNull(apiTokens.revokedAt),
          or(isNull(apiTokens.expiresAt), gt(apiTokens.expiresAt, sql`now()`)),
        ),
      );
    if (activeCount >= MAX_TOKENS_PER_BOT) {
      capDetail = `Bot already has ${activeCount} active tokens (max ${MAX_TOKENS_PER_BOT}). Revoke one before minting another.`;
      return;
    }

    const [inserted] = await tx
      .insert(apiTokens)
      .values({
        userId: botId,
        name: `${TOKEN_NAME_PREFIX} — ${input.name}`,
        displayPrefix,
        hashedSecret: hashed,
        scopes: granted,
        expiresAt: null,
      })
      .returning({ id: apiTokens.id });

    await tx.insert(apiTokenEvents).values({
      tokenId: inserted.id,
      userId: botId,
      event: "mint",
      scopes: granted,
      metadata: {
        displayPrefix,
        actor: "citizen-bot-owner",
        ownerUserId: parentUserId,
        droppedScopes: dropped,
      },
    });
    insertedTokenId = inserted.id;
  });

  if (ownerProblem !== null) {
    return { ok: false, reason: ownerProblem };
  }
  if (capDetail !== null) {
    return { ok: false, reason: "cap_reached", detail: capDetail };
  }
  if (insertedTokenId === null) {
    return {
      ok: false,
      reason: "transient",
      detail: "Mint failed; please retry.",
    };
  }

  return {
    ok: true,
    plaintext,
    displayPrefix,
    grantedScopes: granted,
    droppedScopes: dropped,
  };
}

export {
  CITIZEN_BOT_CAP_PER_PARENT,
  CITIZEN_BOT_USERNAME_SUFFIX,
  composeCitizenBotUsername,
  looksLikeCitizenBot,
} from "./schemas";
export { CITIZEN_SCOPES } from "./scopes";
