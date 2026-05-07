"use server";

import { revalidatePath } from "next/cache";
import { eq, sql } from "drizzle-orm";
import { z } from "zod";

import { auth } from "@/lib/auth";
import { db } from "@/db/client";
import { users } from "@/db/schema";
import {
  MAX_SELF_RENAMES,
  SELF_RENAME_COOLDOWN_MINUTES,
  canSelfRename,
  isReservedUsername,
  isValidUsernameShape,
  normalizeUsername,
} from "@/lib/username";

export type RenameUsernameResult =
  | { ok: true; username: string }
  | {
      ok: false;
      reason:
        | "unauth"
        | "validation"
        | "reserved"
        | "taken"
        | "no-change"
        | "grace_expired"
        | "count_exceeded"
        | "cooldown";
      message?: string;
    };

const input = z.object({
  username: z.string().trim(),
});

export async function renameUsername(
  formData: FormData,
): Promise<RenameUsernameResult> {
  const session = await auth();
  if (!session?.user?.id) return { ok: false, reason: "unauth" };

  const parsed = input.safeParse({ username: formData.get("username") });
  if (!parsed.success) {
    return { ok: false, reason: "validation", message: "Invalid input." };
  }
  const desired = normalizeUsername(parsed.data.username);

  if (!isValidUsernameShape(desired)) {
    return {
      ok: false,
      reason: "validation",
      message:
        "3–24 characters · letters, digits, single dashes · must start and end with a letter or digit.",
    };
  }
  if (isReservedUsername(desired)) {
    return { ok: false, reason: "reserved", message: "That name is reserved." };
  }

  // Pull current row to evaluate self-rename eligibility.
  const [row] = await db
    .select({
      id: users.id,
      username: users.username,
      createdAt: users.createdAt,
      usernameLastChangedAt: users.usernameLastChangedAt,
      selfUsernameRenameCount: users.selfUsernameRenameCount,
    })
    .from(users)
    .where(eq(users.id, session.user.id))
    .limit(1);
  if (!row) return { ok: false, reason: "unauth" };

  if (row.username.toLowerCase() === desired) {
    return { ok: false, reason: "no-change", message: "That's your current username." };
  }

  const decision = canSelfRename({
    createdAt: new Date(row.createdAt),
    selfUsernameRenameCount: row.selfUsernameRenameCount,
    usernameLastChangedAt: row.usernameLastChangedAt
      ? new Date(row.usernameLastChangedAt)
      : null,
  });
  if (!decision.ok) {
    return { ok: false, reason: decision.reason };
  }

  // Atomic claim: only update if (a) the row's eligibility holds at write
  // time (count + cooldown re-checked under DB clock to defend against
  // request reordering), and (b) the target name is free. The unique
  // index on username catches collisions even without the EXISTS clause,
  // but the explicit check produces a cleaner error path.
  const result = await db
    .update(users)
    .set({
      username: desired,
      usernameLastChangedAt: new Date(),
      selfUsernameRenameCount: sql`${users.selfUsernameRenameCount} + 1`,
      updatedAt: new Date(),
    })
    .where(
      sql`${users.id} = ${row.id}
          AND ${users.selfUsernameRenameCount} < ${MAX_SELF_RENAMES}
          AND (${users.usernameLastChangedAt} IS NULL
               OR ${users.usernameLastChangedAt} < NOW() - (${SELF_RENAME_COOLDOWN_MINUTES} || ' minutes')::interval)
          AND NOT EXISTS (
            SELECT 1 FROM ${users} u2 WHERE u2.username = ${desired}
          )`,
    )
    .returning({ id: users.id, username: users.username });

  if (result.length === 0) {
    // The atomic UPDATE failed for one of three reasons. Distinguish
    // them by re-reading instead of guessing — a cooldown false-positive
    // when the user has actually exhausted their renames is misleading.
    const [conflict] = await db
      .select({ id: users.id })
      .from(users)
      .where(eq(users.username, desired))
      .limit(1);
    if (conflict) return { ok: false, reason: "taken", message: "That name is taken." };

    const [fresh] = await db
      .select({
        createdAt: users.createdAt,
        usernameLastChangedAt: users.usernameLastChangedAt,
        selfUsernameRenameCount: users.selfUsernameRenameCount,
      })
      .from(users)
      .where(eq(users.id, row.id))
      .limit(1);
    if (fresh) {
      const second = canSelfRename({
        createdAt: new Date(fresh.createdAt),
        selfUsernameRenameCount: fresh.selfUsernameRenameCount,
        usernameLastChangedAt: fresh.usernameLastChangedAt
          ? new Date(fresh.usernameLastChangedAt)
          : null,
      });
      if (!second.ok) return { ok: false, reason: second.reason };
    }
    return { ok: false, reason: "cooldown" };
  }

  revalidatePath("/settings");
  revalidatePath(`/u/${row.username}`);
  revalidatePath(`/u/${result[0].username}`);

  return { ok: true, username: result[0].username };
}
