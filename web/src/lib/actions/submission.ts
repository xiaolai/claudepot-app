"use server";

import { redirect } from "next/navigation";

import { auth } from "@/lib/auth";
import type { Submission as PrototypeSubmission } from "@/lib/prototype-fixtures";
import {
  createSubmission,
  deleteSubmissionAsAuthor,
  submissionInputSchema,
  updateSubmissionAsAuthor,
  updateSubmissionInputSchema,
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

/* ── editSubmission — thin wrapper over updateSubmissionAsAuthor ─ */

export async function editSubmission(
  input: unknown,
): Promise<
  | { ok: true }
  | { ok: false; reason: "unauth" | "not_found" | "forbidden" | "expired" | "validation" }
> {
  const session = await auth();
  if (!session?.user?.id) return { ok: false, reason: "unauth" };

  // Web action accepts { id, title?, text? } — split id off and pass
  // the rest to the core's narrower schema.
  if (typeof input !== "object" || input === null || !("id" in input)) {
    return { ok: false, reason: "validation" };
  }
  const { id, ...rest } = input as { id: unknown } & Record<string, unknown>;
  if (typeof id !== "string") return { ok: false, reason: "validation" };

  const parsed = updateSubmissionInputSchema.safeParse(rest);
  if (!parsed.success) return { ok: false, reason: "validation" };

  const result = await updateSubmissionAsAuthor(session.user.id, id, parsed.data);
  if (!result.ok) {
    // The core's "noop" is success-equivalent for the web caller —
    // hitting Save with no changes shouldn't error.
    if (result.reason === "noop") return { ok: true };
    // The core's "invalid" reason flags URL/text invariant violations
    // (adding text to a link post, clearing a self-post). Web caller
    // sees them as validation failures.
    if (result.reason === "invalid") return { ok: false, reason: "validation" };
    return { ok: false, reason: result.reason };
  }
  return { ok: true };
}

/* ── deleteSubmission (soft delete tombstone, no replies → hard) ─ */

export async function deleteSubmission(
  id: string,
): Promise<{ ok: true } | { ok: false; reason: "unauth" | "not_found" | "forbidden" }> {
  const session = await auth();
  if (!session?.user?.id) return { ok: false, reason: "unauth" };

  return deleteSubmissionAsAuthor(session.user.id, id);
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
    if (result.reason === "rejected") {
      // Moderator rejected — the row exists with state='rejected'
      // and a notification was written. Send the user to the
      // appeal page rather than the (hidden) post permalink.
      redirect(`/appeal/${result.decisionId}`);
    }
    redirect(`/submit?error=${result.reason}`);
  }
  redirect(`/post/${result.submissionId}`);
}
