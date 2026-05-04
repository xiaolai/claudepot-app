"use server";

import { revalidatePath } from "next/cache";
import { and, count, desc, eq, gt, isNull, or, sql } from "drizzle-orm";
import { z } from "zod";

import { auth } from "@/lib/auth";
import { db } from "@/db/client";
import { apiTokens, apiTokenEvents } from "@/db/schema";
import { generateToken } from "@/lib/api/tokens";
import { SCOPES, normalizeScopes, type Scope } from "@/lib/api/scopes";

/* ── Defaults ───────────────────────────────────────────────────
 *
 * Every PAT expires by default after 180 days. Only `staff` users may
 * mint never-expiring tokens (`expiresInDays === null`). Agent users
 * (role="system") deliberately do NOT count as staff here — agent
 * tokens should be issued FOR them by a real staff member, not by
 * the agents themselves, so a leaked never-expire agent token can't
 * be a self-amplifying attack vector.
 */

const DEFAULT_EXPIRY_DAYS = 180;
const MAX_TOKENS_PER_USER = 20;
const MAX_NAME_LENGTH = 80;

/* ── createApiToken ─────────────────────────────────────────────
 *
 * Plaintext is shown ONCE to the caller and then never recoverable.
 * The /settings/tokens UI is responsible for surfacing it via a
 * one-time flash and warning the user to copy it now.
 */

const createInput = z.object({
  name: z.string().trim().min(1).max(MAX_NAME_LENGTH),
  scopes: z
    .array(z.enum(SCOPES))
    .min(1, "Pick at least one scope.")
    .max(SCOPES.length),
  expiresInDays: z
    .union([z.number().int().min(1).max(3650), z.null()])
    .optional(),
});

export type CreateTokenInput = z.infer<typeof createInput>;

export type CreateTokenResult =
  | {
      ok: true;
      plaintext: string;
      token: {
        id: string;
        name: string;
        displayPrefix: string;
        scopes: Scope[];
        expiresAt: Date | null;
        createdAt: Date;
      };
    }
  | {
      ok: false;
      reason: "unauth" | "validation" | "limit" | "locked";
      detail?: string;
    };

export async function createApiToken(
  input: unknown,
): Promise<CreateTokenResult> {
  const session = await auth();
  if (!session?.user?.id) return { ok: false, reason: "unauth" };
  if (session.user.role === "locked") {
    return {
      ok: false,
      reason: "locked",
      detail: "This account is locked and cannot mint API tokens.",
    };
  }

  const parsed = createInput.safeParse(input);
  if (!parsed.success) {
    return { ok: false, reason: "validation", detail: parsed.error.message };
  }
  // Z.enum allows duplicate entries; collapse before storing.
  const scopes = normalizeScopes(parsed.data.scopes);
  if (scopes.length === 0) {
    return {
      ok: false,
      reason: "validation",
      detail: "Pick at least one scope.",
    };
  }

  // Only `staff` may mint never-expiring tokens. `system` (agent) users
  // are deliberately excluded — see header comment for rationale.
  const canRequestNoExpiry = session.user.role === "staff";
  const expiresInDays =
    parsed.data.expiresInDays === null && canRequestNoExpiry
      ? null
      : (parsed.data.expiresInDays ?? DEFAULT_EXPIRY_DAYS);
  const expiresAt =
    expiresInDays === null
      ? null
      : new Date(Date.now() + expiresInDays * 86_400_000);

  const { plaintext, hashed, displayPrefix } = generateToken();
  const userId = session.user.id;

  // Atomic cap enforcement.
  //
  // The previous shape (SELECT count, INSERT) let two concurrent mints
  // both pass the count and exceed the cap. We can't use SELECT … FOR
  // UPDATE on a single row because there's nothing single-row to lock,
  // and locking the users row would block unrelated traffic
  // (session.updatedAt bumps, karma writes, profile edits).
  //
  // Postgres advisory locks: `pg_advisory_xact_lock(int)` takes a
  // transaction-scoped exclusive lock on an integer key, isolated from
  // anything that doesn't take the same key. We hash a stable namespace
  // string with the user id; mints for different users don't contend,
  // and no other code path takes a lock under the 'api_tokens:mint:*'
  // namespace.
  let limitDetail: string | null = null;
  type MintRow = {
    id: string;
    name: string;
    displayPrefix: string;
    scopes: string[];
    expiresAt: Date | null;
    createdAt: Date;
  };
  let row: MintRow | null = null;
  await db.transaction(async (tx) => {
    await tx.execute(
      sql`SELECT pg_advisory_xact_lock(hashtext(${"api_tokens:mint:" + userId}))`,
    );

    const [{ n: activeCount = 0 } = { n: 0 }] = await tx
      .select({ n: count() })
      .from(apiTokens)
      .where(
        and(
          eq(apiTokens.userId, userId),
          isNull(apiTokens.revokedAt),
          or(isNull(apiTokens.expiresAt), gt(apiTokens.expiresAt, sql`now()`)),
        ),
      );
    if (activeCount >= MAX_TOKENS_PER_USER) {
      limitDetail = `You already have ${activeCount} active tokens (max ${MAX_TOKENS_PER_USER}). Revoke one before minting another.`;
      return;
    }

    const [inserted] = await tx
      .insert(apiTokens)
      .values({
        userId,
        name: parsed.data.name,
        displayPrefix,
        hashedSecret: hashed,
        scopes,
        expiresAt,
      })
      .returning({
        id: apiTokens.id,
        name: apiTokens.name,
        displayPrefix: apiTokens.displayPrefix,
        scopes: apiTokens.scopes,
        expiresAt: apiTokens.expiresAt,
        createdAt: apiTokens.createdAt,
      });
    row = {
      id: inserted.id,
      name: inserted.name,
      displayPrefix: inserted.displayPrefix,
      scopes: inserted.scopes as string[],
      expiresAt: inserted.expiresAt,
      createdAt: inserted.createdAt,
    };

    // Audit event lives in the same transaction so a leak between mint
    // and event is impossible. Failure here aborts the mint, which is
    // what we want for auditability — losing the event row would leave
    // an unaudited live token.
    await tx.insert(apiTokenEvents).values({
      tokenId: inserted.id,
      userId,
      event: "mint",
      scopes,
      metadata: {
        displayPrefix,
        expiresAt: expiresAt ? expiresAt.toISOString() : null,
      },
    });
  });

  if (limitDetail !== null) {
    return { ok: false, reason: "limit", detail: limitDetail };
  }
  if (row === null) {
    // Belt-and-suspenders: transaction rolled back without setting either
    // limitDetail or row. Treat as a transient failure rather than ok.
    return {
      ok: false,
      reason: "validation",
      detail: "Mint failed; please retry.",
    };
  }
  // After the assignment above, TypeScript still narrows row to never;
  // pin the type to MintRow before reading fields.
  const minted: MintRow = row;

  revalidatePath("/settings/tokens");

  return {
    ok: true,
    plaintext,
    token: {
      id: minted.id,
      name: minted.name,
      displayPrefix: minted.displayPrefix,
      scopes: minted.scopes as Scope[],
      expiresAt: minted.expiresAt,
      createdAt: minted.createdAt,
    },
  };
}

/* ── revokeApiToken ─────────────────────────────────────────────
 *
 * Sets revoked_at; the row stays for audit. Idempotent — re-revoking
 * an already-revoked token returns ok (no-op). A user can only revoke
 * their own tokens; revoking someone else's id returns not_found.
 */

const revokeInput = z.object({ tokenId: z.string().uuid() });

export type RevokeTokenResult =
  | { ok: true }
  | { ok: false; reason: "unauth" | "validation" | "not_found" };

export async function revokeApiToken(
  input: unknown,
): Promise<RevokeTokenResult> {
  const session = await auth();
  if (!session?.user?.id) return { ok: false, reason: "unauth" };

  const parsed = revokeInput.safeParse(input);
  if (!parsed.success) return { ok: false, reason: "validation" };

  // Atomic flip: UPDATE ... WHERE revoked_at IS NULL. Only the call
  // that actually flips the bit gets a row back from RETURNING; a
  // concurrent revoke arriving microseconds later sees an empty
  // result. This is the single-source-of-truth signal for "did we
  // just revoke" — only that caller emits the audit-log row, so
  // concurrent revokes don't produce duplicate "revoke" events.
  const flipped = await db
    .update(apiTokens)
    .set({ revokedAt: new Date() })
    .where(
      and(
        eq(apiTokens.id, parsed.data.tokenId),
        eq(apiTokens.userId, session.user.id),
        isNull(apiTokens.revokedAt),
      ),
    )
    .returning({ id: apiTokens.id });

  if (flipped.length === 1) {
    // Best-effort audit event. A failure here should not abort the
    // revoke — the bit is already flipped.
    try {
      await db.insert(apiTokenEvents).values({
        tokenId: parsed.data.tokenId,
        userId: session.user.id,
        event: "revoke",
      });
    } catch (err) {
      console.error("[api-tokens] audit-log revoke failed:", err);
    }
    revalidatePath("/settings/tokens");
    return { ok: true };
  }

  // Empty result. Distinguish "doesn't exist or not yours"
  // (→ not_found) from "already revoked" (→ ok, idempotent).
  const [existing] = await db
    .select({ id: apiTokens.id })
    .from(apiTokens)
    .where(
      and(
        eq(apiTokens.id, parsed.data.tokenId),
        eq(apiTokens.userId, session.user.id),
      ),
    )
    .limit(1);
  if (!existing) return { ok: false, reason: "not_found" };
  return { ok: true };
}

/* ── listMyApiTokens ────────────────────────────────────────────
 *
 * Returns ACTIVE tokens only — not revoked, not expired. The full
 * lifecycle history (mints, revokes) is kept in api_token_events and
 * surfaced separately if/when an audit view is added; the user-facing
 * list is for "tokens I currently have", so dead rows would just be
 * clutter.
 */

export type TokenListItem = {
  id: string;
  name: string;
  displayPrefix: string;
  scopes: Scope[];
  lastUsedAt: Date | null;
  expiresAt: Date | null;
  createdAt: Date;
};

export async function listMyApiTokens(): Promise<TokenListItem[]> {
  const session = await auth();
  if (!session?.user?.id) return [];

  const rows = await db
    .select({
      id: apiTokens.id,
      name: apiTokens.name,
      displayPrefix: apiTokens.displayPrefix,
      scopes: apiTokens.scopes,
      lastUsedAt: apiTokens.lastUsedAt,
      expiresAt: apiTokens.expiresAt,
      createdAt: apiTokens.createdAt,
    })
    .from(apiTokens)
    .where(
      and(
        eq(apiTokens.userId, session.user.id),
        isNull(apiTokens.revokedAt),
        or(isNull(apiTokens.expiresAt), gt(apiTokens.expiresAt, sql`now()`)),
      ),
    )
    .orderBy(desc(apiTokens.createdAt));

  return rows.map((r) => ({ ...r, scopes: r.scopes as Scope[] }));
}

/* ── useActionState wrappers for the /settings/tokens UI ────────
 *
 * Thin form-action shims around the typed actions above. Forms call
 * these so React 19's useActionState can hold the result (plaintext on
 * mint success; flash messages on either action). The typed callers
 * (CLI scripts, future programmatic mints) still use the originals.
 */

export type MintFormState =
  | { phase: "idle" }
  | { phase: "ok"; plaintext: string; tokenName: string; displayPrefix: string }
  | { phase: "err"; message: string };

export async function mintApiTokenFormAction(
  _prev: MintFormState,
  formData: FormData,
): Promise<MintFormState> {
  const scopes = formData.getAll("scopes").map(String);
  const expiresRaw = String(formData.get("expiresInDays") ?? "");
  const expiresInDays =
    expiresRaw === "never"
      ? null
      : expiresRaw === ""
        ? undefined
        : Number(expiresRaw);

  const result = await createApiToken({
    name: formData.get("name"),
    scopes,
    expiresInDays,
  });

  if (!result.ok) {
    const map: Record<typeof result.reason, string> = {
      unauth: "Sign in first.",
      validation: result.detail ?? "Check the name, scopes, and expiry.",
      limit: result.detail ?? "Token limit reached.",
      locked: "Account locked.",
    };
    return { phase: "err", message: map[result.reason] };
  }

  return {
    phase: "ok",
    plaintext: result.plaintext,
    tokenName: result.token.name,
    displayPrefix: result.token.displayPrefix,
  };
}

export type RevokeFormState =
  | { phase: "idle" }
  | { phase: "ok"; message: string }
  | { phase: "err"; message: string };

export async function revokeApiTokenFormAction(
  _prev: RevokeFormState,
  formData: FormData,
): Promise<RevokeFormState> {
  const result = await revokeApiToken({ tokenId: formData.get("tokenId") });
  if (result.ok) return { phase: "ok", message: "Token revoked." };
  const map: Record<typeof result.reason, string> = {
    unauth: "Sign in first.",
    validation: "Bad token id.",
    not_found: "Already revoked or not yours.",
  };
  return { phase: "err", message: map[result.reason] };
}
