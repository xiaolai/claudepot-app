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

    // Distinct enum values added in migration 0019 — /admin/log can
    // now filter on the action discriminator without parsing the
    // free-text note. Note still carries the target user id so the
    // log row is self-describing.
    await tx.insert(moderationLog).values({
      staffId,
      action: exempt ? "bot_exempt_grant" : "bot_exempt_revoke",
      targetType: "user",
      targetId: userId,
      note: null,
    });
  });

  revalidatePath("/admin/console/users");
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

/* ── Monthly USD cap (migration 0028 + 0030) ──────────────────── */

const setCapInput = z.object({
  userId: z.uuid(),
  /** Empty string clears the cap. Otherwise must be ≥ 0 and ≤ 1M. */
  cap: z
    .string()
    .trim()
    .transform((v) => (v === "" ? null : Number.parseFloat(v)))
    .refine(
      (v) => v === null || (Number.isFinite(v) && v >= 0 && v <= 1_000_000),
      "Cap must be a non-negative number ≤ 1,000,000.",
    ),
});

export type SetCapResult =
  | { ok: true; cap: number | null }
  | {
      ok: false;
      reason: "forbidden" | "validation" | "not_found" | "not_agent";
    };

/**
 * Set or clear users.monthly_usd_cap. Empty input clears the cap
 * (back to null = no cap). Asserts target is is_agent=true; the
 * cap has no meaning for human accounts and persistBotReport's
 * cap-breach detection only fires on bot tokens anyway.
 *
 * Each set/clear writes a moderation_log row tagged with the
 * cap value so /admin/log shows the audit trail.
 */
export async function setBotMonthlyCap(
  input: unknown,
): Promise<SetCapResult> {
  const staffId = await requireStaffId();
  if (!staffId) return { ok: false, reason: "forbidden" };

  const parsed = setCapInput.safeParse(input);
  if (!parsed.success) return { ok: false, reason: "validation" };

  const [target] = await db
    .select({ isAgent: users.isAgent })
    .from(users)
    .where(eq(users.id, parsed.data.userId))
    .limit(1);
  if (!target) return { ok: false, reason: "not_found" };
  if (!target.isAgent) return { ok: false, reason: "not_agent" };

  const capStr =
    parsed.data.cap === null ? null : parsed.data.cap.toFixed(2);

  await db.transaction(async (tx) => {
    await tx
      .update(users)
      .set({ monthlyUsdCap: capStr })
      .where(eq(users.id, parsed.data.userId));

    await tx.insert(moderationLog).values({
      staffId,
      action: capStr === null ? "bot_cap_clear" : "bot_cap_set",
      targetType: "user",
      targetId: parsed.data.userId,
      // Log the new cap value in the note so the audit row is
      // self-describing; the action discriminator already says
      // set/clear, but the amount is the load-bearing detail.
      note: capStr === null ? null : `cap=$${capStr}`,
    });
  });

  revalidatePath("/admin/console/users");
  revalidatePath("/admin/log");
  return { ok: true, cap: parsed.data.cap };
}

export type SetCapActionState =
  | { ok: true; message: string }
  | { ok: false; message: string };

export async function setBotMonthlyCapFormAction(
  _prev: SetCapActionState,
  formData: FormData,
): Promise<SetCapActionState> {
  const result = await setBotMonthlyCap({
    userId: formData.get("userId"),
    cap: String(formData.get("cap") ?? ""),
  });
  if (result.ok) {
    return {
      ok: true,
      message:
        result.cap === null
          ? "Cap cleared."
          : `Cap set: $${result.cap.toFixed(2)}.`,
    };
  }
  switch (result.reason) {
    case "forbidden":
      return { ok: false, message: "Not authorized." };
    case "not_found":
      return { ok: false, message: "User not found." };
    case "not_agent":
      return { ok: false, message: "Only bot accounts have caps." };
    case "validation":
      return { ok: false, message: "Cap must be 0–1,000,000 or empty." };
  }
}
