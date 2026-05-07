"use server";

import { revalidatePath } from "next/cache";
import { and, eq } from "drizzle-orm";
import { z } from "zod";

import { db } from "@/db/client";
import { botReports, moderationLog } from "@/db/schema";
import { requireStaffId } from "@/lib/staff";

/**
 * Resolve a bot proposal — staff acks or rejects a proposal that a
 * bot posted via POST /api/v1/bots/reports with kind=proposal.
 *
 * Closing a proposal:
 *   - flips bot_reports.status from 'open' → 'accepted'|'rejected'
 *   - stamps resolved_by + resolved_at
 *   - writes a moderation_log row tagged 'bot_proposal_<verdict>'
 *
 * The partial unique index on (bot_id, payload->>'key') WHERE
 * status='open' implicitly clears once status flips, so a follow-up
 * proposal with the same key can land cleanly. That's the design:
 * "open" means "blocking ack"; resolved means "history".
 *
 * Acting on the proposal does NOT itself execute the change the
 * bot proposed (e.g. accepting a vocab_tag proposal does not add
 * the tag to the vocabulary — staff goes to /admin/console/vocabulary
 * to do that). The proposal is the bot's flag; the staff decision
 * here is the ack, not the action. Combining both would conflate
 * "I saw this" with "I want this," and the bots' proposal kinds
 * are heterogeneous enough that each needs its own real action
 * surface.
 */

const actInput = z.object({
  reportId: z.uuid(),
  action: z.enum(["accept", "reject"]),
});

export type ProposalActionResult =
  | { ok: true; status: "accepted" | "rejected" }
  | {
      ok: false;
      reason: "forbidden" | "validation" | "not_found" | "not_open";
    };

export async function actOnBotProposal(
  input: unknown,
): Promise<ProposalActionResult> {
  const staffId = await requireStaffId();
  if (!staffId) return { ok: false, reason: "forbidden" };

  const parsed = actInput.safeParse(input);
  if (!parsed.success) return { ok: false, reason: "validation" };
  const { reportId, action } = parsed.data;

  const newStatus = action === "accept" ? "accepted" : "rejected";

  const result = await db.transaction(async (tx) => {
    // Conditional update — only flips an open proposal, never an
    // already-resolved one. The returning row tells us whether the
    // status change actually took effect.
    const [updated] = await tx
      .update(botReports)
      .set({
        status: newStatus,
        resolvedBy: staffId,
        resolvedAt: new Date(),
      })
      .where(
        and(
          eq(botReports.id, reportId),
          eq(botReports.kind, "proposal"),
          eq(botReports.status, "open"),
        ),
      )
      .returning({ id: botReports.id, botId: botReports.botId });

    if (!updated) return null;

    await tx.insert(moderationLog).values({
      staffId,
      action: "approve",
      targetType: "user",
      targetId: updated.botId,
      note: `bot_proposal_${newStatus}:${reportId}`,
    });

    return updated;
  });

  if (!result) {
    // Either the row doesn't exist, isn't a proposal, or has
    // already been resolved. Treat the last two as not_open so the
    // UI can render a quiet "already actioned" rather than a 500.
    const [exists] = await db
      .select({ status: botReports.status, kind: botReports.kind })
      .from(botReports)
      .where(eq(botReports.id, reportId))
      .limit(1);
    if (!exists) return { ok: false, reason: "not_found" };
    return { ok: false, reason: "not_open" };
  }

  revalidatePath("/admin");
  revalidatePath("/admin/console/bots");
  revalidatePath("/admin/log");
  return { ok: true, status: newStatus };
}

export type ProposalActionState = { ok: boolean; message: string };

export async function proposalActionForm(
  _prev: ProposalActionState,
  formData: FormData,
): Promise<ProposalActionState> {
  const result = await actOnBotProposal({
    reportId: formData.get("reportId"),
    action: formData.get("action"),
  });
  if (result.ok) {
    return {
      ok: true,
      message:
        result.status === "accepted" ? "Accepted." : "Rejected.",
    };
  }
  switch (result.reason) {
    case "forbidden":
      return { ok: false, message: "Not authorized." };
    case "not_found":
      return { ok: false, message: "Proposal not found." };
    case "not_open":
      return {
        ok: false,
        message: "Already actioned by someone else.",
      };
    case "validation":
      return { ok: false, message: "Invalid request." };
  }
}
