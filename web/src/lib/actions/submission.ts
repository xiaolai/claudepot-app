"use server";

import { revalidatePath } from "next/cache";
import { redirect } from "next/navigation";
import { eq } from "drizzle-orm";
import { z } from "zod";

import { auth } from "@/lib/auth";
import { db } from "@/db/client";
import { submissions } from "@/db/schema";
import type { Submission as PrototypeSubmission } from "@/lib/prototype-fixtures";
import {
  createSubmission,
  submissionInputSchema,
  type SubmissionInput,
  type SubmitResult as CoreSubmitResult,
} from "@/lib/submissions";

export type { SubmissionInput } from "@/lib/submissions";

// Server-action result includes "unauth" — the API/MCP surfaces handle
// auth before they ever call createSubmission, so they never see it.
export type SubmitResult =
  | CoreSubmitResult
  | { ok: false; reason: "unauth"; detail?: string };

/* ── submitPost (web UI server action) ─────────────────────────── */

export async function submitPost(input: unknown): Promise<SubmitResult> {
  const session = await auth();
  if (!session?.user?.id) return { ok: false, reason: "unauth" };

  const parsed = submissionInputSchema.safeParse(input);
  if (!parsed.success) {
    return { ok: false, reason: "validation", detail: parsed.error.message };
  }

  return createSubmission(session.user.id, parsed.data, { surface: "web" });
}

/* ── editSubmission (5-minute window) ──────────────────────────── */

const EDIT_WINDOW_MS = 5 * 60 * 1000;

const editInput = z.object({
  id: z.uuid(),
  title: z.string().trim().min(3).max(120).optional(),
  text: z.string().trim().max(40_000).optional(),
});

export async function editSubmission(
  input: unknown,
): Promise<
  | { ok: true }
  | { ok: false; reason: "unauth" | "not_found" | "forbidden" | "expired" | "validation" }
> {
  const session = await auth();
  if (!session?.user?.id) return { ok: false, reason: "unauth" };

  const parsed = editInput.safeParse(input);
  if (!parsed.success) return { ok: false, reason: "validation" };

  const [existing] = await db
    .select({
      authorId: submissions.authorId,
      createdAt: submissions.createdAt,
    })
    .from(submissions)
    .where(eq(submissions.id, parsed.data.id))
    .limit(1);
  if (!existing) return { ok: false, reason: "not_found" };
  if (existing.authorId !== session.user.id) return { ok: false, reason: "forbidden" };
  if (Date.now() - existing.createdAt.getTime() > EDIT_WINDOW_MS)
    return { ok: false, reason: "expired" };

  const updates: Record<string, unknown> = {};
  if (parsed.data.title !== undefined) updates.title = parsed.data.title;
  if (parsed.data.text !== undefined) updates.text = parsed.data.text;
  if (Object.keys(updates).length === 0) return { ok: true };

  await db.update(submissions).set(updates).where(eq(submissions.id, parsed.data.id));
  revalidatePath(`/post/${parsed.data.id}`);
  return { ok: true };
}

/* ── deleteSubmission (soft delete tombstone, no replies → hard) ─ */

export async function deleteSubmission(
  id: string,
): Promise<{ ok: true } | { ok: false; reason: "unauth" | "not_found" | "forbidden" }> {
  const session = await auth();
  if (!session?.user?.id) return { ok: false, reason: "unauth" };

  const [existing] = await db
    .select({ authorId: submissions.authorId })
    .from(submissions)
    .where(eq(submissions.id, id))
    .limit(1);
  if (!existing) return { ok: false, reason: "not_found" };
  if (existing.authorId !== session.user.id) return { ok: false, reason: "forbidden" };

  await db
    .update(submissions)
    .set({ deletedAt: new Date() })
    .where(eq(submissions.id, id));
  revalidatePath(`/post/${id}`);
  revalidatePath("/");
  return { ok: true };
}

/* ── Helper exposed to UI: redirect after submit ───────────────── */

export async function submitAndRedirect(formData: FormData) {
  const tags = formData
    .getAll("tags")
    .map(String)
    .filter(Boolean);

  const input: SubmissionInput = {
    type: (formData.get("type") as PrototypeSubmission["type"]) ?? "discussion",
    title: String(formData.get("title") ?? ""),
    url: String(formData.get("url") ?? "") || undefined,
    text: String(formData.get("text") ?? "") || undefined,
    tags,
  };

  const result = await submitPost(input);
  if (!result.ok) {
    if (result.reason === "duplicate") {
      redirect(`/post/${result.existingId}?dup=1`);
    }
    redirect(`/submit?error=${result.reason}`);
  }
  redirect(`/post/${result.submissionId}`);
}
