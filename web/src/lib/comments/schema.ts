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

export const updateCommentInputSchema = z.object({
  body: z.string().trim().min(2).max(40_000),
});

export type UpdateCommentInput = z.infer<typeof updateCommentInputSchema>;

export type UpdateCommentResult =
  | { ok: true; silent: boolean; submissionId: string; updatedAt: Date | null }
  | {
      ok: false;
      reason: "not_found" | "forbidden" | "expired" | "noop";
    };
