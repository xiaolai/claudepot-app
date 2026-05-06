/**
 * Author-only soft delete (sets `deleted_at`). Used by both the web
 * server action and the REST DELETE endpoint. Auth happens at each
 * caller — this function only enforces "the supplied authorId owns
 * the row".
 */

import { revalidatePath } from "next/cache";
import { eq } from "drizzle-orm";

import { db } from "@/db/client";
import { submissions } from "@/db/schema";
import type { DeleteSubmissionResult } from "./schema";

export async function deleteSubmissionAsAuthor(
  authorId: string,
  submissionId: string,
): Promise<DeleteSubmissionResult> {
  const [existing] = await db
    .select({ authorId: submissions.authorId })
    .from(submissions)
    .where(eq(submissions.id, submissionId))
    .limit(1);
  if (!existing) return { ok: false, reason: "not_found" };
  if (existing.authorId !== authorId) return { ok: false, reason: "forbidden" };

  await db
    .update(submissions)
    .set({ deletedAt: new Date() })
    .where(eq(submissions.id, submissionId));
  revalidatePath(`/post/${submissionId}`);
  revalidatePath("/");
  return { ok: true };
}
