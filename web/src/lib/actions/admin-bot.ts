"use server";

import { revalidatePath } from "next/cache";
import { eq } from "drizzle-orm";
import { z } from "zod";

import { db } from "@/db/client";
import { moderationLog, users } from "@/db/schema";
import { requireStaffId } from "@/lib/staff";

/**
 * Toggle the bot_moderation_exempt flag on a user row.
 *
 * Staff-only. Asserts target user has is_agent=true — non-bot
 * accounts cannot be exempted (matches the runtime assert in
 * lib/moderation/exempt.ts). The /admin/users UI hides the toggle
 * for non-agent rows, but server-side validation is the load-bearing
 * check.
 *
 * Each toggle writes a moderation_log row with action='approve'
 * (existing enum) and a note tagged 'bot_exempt_grant:<userId>' or
 * 'bot_exempt_revoke:<userId>' so /admin/log shows the change.
 */

const toggleInput = z.object({
  userId: z.uuid(),
  exempt: z.enum(["true", "false"]).transform((v) => v === "true"),
});

export type BotExemptResult =
  | { ok: true; exempt: boolean }
  | {
      ok: false;
      reason: "forbidden" | "validation" | "not_found" | "not_agent";
    };

export async function toggleBotModerationExempt(
  input: unknown,
): Promise<BotExemptResult> {
  const staffId = await requireStaffId();
  if (!staffId) return { ok: false, reason: "forbidden" };

  const parsed = toggleInput.safeParse(input);
  if (!parsed.success) return { ok: false, reason: "validation" };

  const { userId, exempt } = parsed.data;

  const [target] = await db
    .select({ isAgent: users.isAgent })
    .from(users)
    .where(eq(users.id, userId))
    .limit(1);

  if (!target) return { ok: false, reason: "not_found" };
  if (!target.isAgent) return { ok: false, reason: "not_agent" };

  await db.transaction(async (tx) => {
    await tx
      .update(users)
      .set({ botModerationExempt: exempt })
      .where(eq(users.id, userId));

    await tx.insert(moderationLog).values({
      staffId,
      action: "approve",
      targetType: null,
      targetId: null,
      note: exempt
        ? `bot_exempt_grant:${userId}`
        : `bot_exempt_revoke:${userId}`,
    });
  });

  revalidatePath("/admin/users");
  revalidatePath("/admin/log");
  return { ok: true, exempt };
}

export type BotExemptActionState = { ok: boolean; message: string };

export async function botExemptFormAction(
  _prev: BotExemptActionState,
  formData: FormData,
): Promise<BotExemptActionState> {
  const result = await toggleBotModerationExempt({
    userId: formData.get("userId"),
    exempt: formData.get("exempt"),
  });
  if (result.ok) {
    return {
      ok: true,
      message: result.exempt ? "Exempt." : "Now moderated.",
    };
  }
  switch (result.reason) {
    case "forbidden":
      return { ok: false, message: "Not authorized." };
    case "not_found":
      return { ok: false, message: "User not found." };
    case "not_agent":
      return { ok: false, message: "Only bot accounts can be exempted." };
    case "validation":
      return { ok: false, message: "Invalid request." };
  }
}
