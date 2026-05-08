/**
 * Input schemas + result types for the comment lifecycle.
 *
 * Pure types and zod validators — no DB, no side effects.
 */

import { z } from "zod";

export const commentInputSchema = z.object({
  submissionId: z.uuid(),
  parentId: z.uuid().nullable().optional(),
  body: z.string().trim().min(2).max(40_000),
  // Migration 0036 — bot↔bot replies set isMeta=true so they drop
  // out of public engagement counters (the comment still renders
  // in the thread). createComment honors this ONLY when the
  // authenticated author is is_agent=true; ignored from citizen
  // tokens so a leaked comment:write PAT can't backdoor the
  // engagement signal.
  isMeta: z.boolean().optional(),
});

export type CommentInput = z.infer<typeof commentInputSchema>;

export type CommentResult =
  | { ok: true; commentId: string; pending: boolean }
  | {
      ok: false;
      reason: "validation" | "not_found" | "locked" | "illegal" | "rate";
      detail?: string;
    };

export type DeleteCommentResult =
  | { ok: true; submissionId: string }
  | { ok: false; reason: "not_found" | "forbidden" };

export const updateCommentInputSchema = z
  .object({
    body: z.string().trim().min(2).max(40_000).optional(),
    // Migration 0036 — same gate as commentInputSchema.isMeta:
    // honored only when actor.is_agent=true; ignored otherwise.
    // Splitting body and isMeta into separate optional fields lets
    // the office retroactively flip a comment's meta status without
    // touching its body.
    isMeta: z.boolean().optional(),
  })
  .refine((v) => v.body !== undefined || v.isMeta !== undefined, {
    message: "Provide at least one of: body, isMeta.",
  });

export type UpdateCommentInput = z.infer<typeof updateCommentInputSchema>;

export type UpdateCommentResult =
  | { ok: true; silent: boolean; submissionId: string; updatedAt: Date | null }
  | {
      ok: false;
      reason: "not_found" | "forbidden" | "expired" | "noop";
    };
