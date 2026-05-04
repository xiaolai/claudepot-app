"use server";

import { revalidatePath } from "next/cache";
import { eq } from "drizzle-orm";
import { z } from "zod";

import { auth } from "@/lib/auth";
import { db } from "@/db/client";
import {
  comments,
  flags,
  moderationLog,
  sessions,
  submissions,
  users,
} from "@/db/schema";
import { requireStaffId } from "@/lib/staff";

/* ── flag (any signed-in user with email_verified) ─────────────── */

const flagInput = z.object({
  targetType: z.enum(["submission", "comment"]),
  targetId: z.uuid(),
  reason: z.string().trim().min(3).max(500),
});

export async function flag(
  input: unknown,
): Promise<
  | { ok: true; flagId: string }
  | { ok: false; reason: "unauth" | "validation" | "unverified" }
> {
  const session = await auth();
  if (!session?.user?.id) return { ok: false, reason: "unauth" };

  const [me] = await db
    .select({ verified: users.emailVerified, role: users.role })
    .from(users)
    .where(eq(users.id, session.user.id))
    .limit(1);
  if (!me?.verified && me?.role !== "system") {
    return { ok: false, reason: "unverified" };
  }

  const parsed = flagInput.safeParse(input);
  if (!parsed.success) return { ok: false, reason: "validation" };

  const [row] = await db
    .insert(flags)
    .values({
      reporterId: session.user.id,
      targetType: parsed.data.targetType,
      targetId: parsed.data.targetId,
      reason: parsed.data.reason,
    })
    .returning({ id: flags.id });
  return { ok: true, flagId: row.id };
}

/* ── Staff actions ─────────────────────────────────────────────── */

const STAFF_ACTIONS = [
  "lock",
  "unlist",
  "delete",
  "restore",
  "dismiss_flag",
  "lock_user",
  "approve",
  "reject",
] as const;

const modInput = z.object({
  action: z.enum(STAFF_ACTIONS),
  targetType: z.enum(["submission", "comment"]).optional(),
  targetId: z.uuid(),
  flagId: z.uuid().optional(),
  note: z.string().trim().max(500).optional(),
});

export type ModResult =
  | { ok: true }
  | { ok: false; reason: "unauth" | "forbidden" | "validation" | "not_found" };

export async function moderationAction(
  input: unknown,
): Promise<ModResult> {
  const staffId = await requireStaffId();
  if (!staffId) return { ok: false, reason: "forbidden" };

  const parsed = modInput.safeParse(input);
  if (!parsed.success) return { ok: false, reason: "validation" };
  const { action, targetType, targetId, flagId, note } = parsed.data;

  // Each branch returns the count of rows it actually mutated. If the
  // target row didn't exist (deleted concurrently, malformed id, wrong
  // targetType for the action), we return not_found and skip the log
  // write — otherwise moderation_log accumulates phantom rows.
  //
  // The mutation AND the log insert are wrapped in a single transaction
  // so a content-state change cannot be visible without a matching audit
  // row, and a failed log insert rolls back the mutation. lock_user's
  // session purge runs inside the same tx for the same reason.
  const applied = await db.transaction(async (tx) => {
    let mutated = 0;
    switch (action) {
      case "delete":
        if (targetType === "submission") {
          const r = await tx
            .update(submissions)
            .set({ deletedAt: new Date() })
            .where(eq(submissions.id, targetId))
            .returning({ id: submissions.id });
          mutated = r.length;
        } else if (targetType === "comment") {
          const r = await tx
            .update(comments)
            .set({ deletedAt: new Date() })
            .where(eq(comments.id, targetId))
            .returning({ id: comments.id });
          mutated = r.length;
        }
        break;
      case "restore":
        // Restore clears all suppression flags: deleted, locked, unlisted.
        if (targetType === "submission") {
          const r = await tx
            .update(submissions)
            .set({ deletedAt: null, lockedAt: null, unlistedAt: null })
            .where(eq(submissions.id, targetId))
            .returning({ id: submissions.id });
          mutated = r.length;
        } else if (targetType === "comment") {
          const r = await tx
            .update(comments)
            .set({ deletedAt: null })
            .where(eq(comments.id, targetId))
            .returning({ id: comments.id });
          mutated = r.length;
        }
        break;
      case "approve":
        if (targetType === "submission") {
          const r = await tx
            .update(submissions)
            .set({ state: "approved", publishedAt: new Date() })
            .where(eq(submissions.id, targetId))
            .returning({ id: submissions.id });
          mutated = r.length;
        } else if (targetType === "comment") {
          const r = await tx
            .update(comments)
            .set({ state: "approved" })
            .where(eq(comments.id, targetId))
            .returning({ id: comments.id });
          mutated = r.length;
        }
        break;
      case "reject":
        if (targetType === "submission") {
          const r = await tx
            .update(submissions)
            .set({ state: "rejected" })
            .where(eq(submissions.id, targetId))
            .returning({ id: submissions.id });
          mutated = r.length;
        } else if (targetType === "comment") {
          const r = await tx
            .update(comments)
            .set({ state: "rejected" })
            .where(eq(comments.id, targetId))
            .returning({ id: comments.id });
          mutated = r.length;
        }
        break;
      case "lock":
        // Audit finding 3.3 — lock now actually blocks comments via
        // locked_at. submitComment checks this column before inserting.
        if (targetType === "submission") {
          const r = await tx
            .update(submissions)
            .set({ lockedAt: new Date() })
            .where(eq(submissions.id, targetId))
            .returning({ id: submissions.id });
          mutated = r.length;
        }
        break;
      case "unlist":
        // Audit finding 3.3 — unlist hides from feeds but keeps permalink
        // accessible. queries.ts feed reads filter on unlisted_at IS NULL.
        if (targetType === "submission") {
          const r = await tx
            .update(submissions)
            .set({ unlistedAt: new Date() })
            .where(eq(submissions.id, targetId))
            .returning({ id: submissions.id });
          mutated = r.length;
        }
        break;
      case "dismiss_flag":
        if (flagId) {
          const r = await tx
            .update(flags)
            .set({
              status: "resolved",
              resolvedBy: staffId,
              resolvedAt: new Date(),
            })
            .where(eq(flags.id, flagId))
            .returning({ id: flags.id });
          mutated = r.length;
        }
        break;
      case "lock_user": {
        // targetId is interpreted as the user id here.
        const r = await tx
          .update(users)
          .set({ role: "locked" })
          .where(eq(users.id, targetId))
          .returning({ id: users.id });
        mutated = r.length;
        if (mutated > 0) {
          // Revoke all live sessions immediately. Inside the same
          // transaction so a session-delete failure rolls back the
          // role flip — otherwise a locked user would keep their
          // existing sessions alive with no audit-visible reason.
          await tx
            .delete(sessions)
            .where(eq(sessions.userId, targetId));
        }
        break;
      }
    }

    if (mutated === 0) return 0;

    await tx.insert(moderationLog).values({
      staffId: staffId,
      action,
      targetType: targetType ?? null,
      targetId: targetId ?? null,
      note: note ?? null,
    });

    return mutated;
  });

  if (applied === 0) return { ok: false, reason: "not_found" };

  revalidatePath("/admin/queue");
  revalidatePath("/admin/log");
  if (targetType === "submission") revalidatePath(`/post/${targetId}`);
  if (action === "lock_user") revalidatePath("/admin/users");
  return { ok: true };
}

/** Discriminated state shape for useActionState consumers. Mirrors the
 *  TagActionState shape in admin-tag.ts so client components can share
 *  the same render pattern. Types-only export is safe inside "use
 *  server" — types are erased at compile time. */
export type ModActionState = { ok: boolean; message: string };

/** FormData adapter for moderationAction. Designed for React 19's
 *  useActionState: receives prior state + FormData, returns a fresh
 *  state. Maps the typed ModResult discriminated union onto a flat
 *  { ok, message } shape so the client can render success/error
 *  messages without inspecting reason codes. */
export async function moderationFormAction(
  _prev: ModActionState,
  formData: FormData,
): Promise<ModActionState> {
  const action = formData.get("action");
  const result = await moderationAction({
    action,
    targetType: formData.get("targetType") || undefined,
    targetId: formData.get("targetId"),
    flagId: formData.get("flagId") || undefined,
    note: formData.get("note") || undefined,
  });
  if (result.ok) {
    return { ok: true, message: successMessage(action) };
  }
  return {
    ok: false,
    message:
      result.reason === "forbidden"
        ? "Not authorized."
        : result.reason === "validation"
          ? "Invalid request."
          : result.reason === "not_found"
            ? "Target no longer exists."
            : "Unauthenticated.",
  };
}

function successMessage(action: unknown): string {
  if (typeof action !== "string") return "Done.";
  switch (action) {
    case "approve": return "Approved.";
    case "reject":  return "Rejected.";
    case "delete":  return "Removed.";
    case "restore": return "Restored.";
    case "lock":    return "Locked.";
    case "unlist":  return "Unlisted.";
    case "dismiss_flag": return "Dismissed.";
    case "lock_user":    return "Suspended.";
    default: return "Done.";
  }
}
