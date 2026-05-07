/**
 * Input schemas + result types for the submission lifecycle.
 *
 * Pure types and zod validators — no DB, no side effects. Owned by
 * the domain layer (lib/submissions/) so the action layer
 * (lib/actions/submission.ts) and the REST surface can both
 * `import { submissionInputSchema, SubmitResult } from "@/lib/submissions"`
 * without pulling create/edit/delete bodies along.
 */

import { z } from "zod";

export const SUBMISSION_TYPES = [
  "news",
  "tip",
  "tutorial",
  "course",
  "article",
  "podcast",
  "interview",
  "tool",
  "discussion",
  // editorial/rubric.yml v0.2.3 types — added in 0008_editorial_runtime
  "release",
  "paper",
  "workflow",
  "case_study",
  "prompt_pattern",
] as const;

export const submissionInputSchema = z
  .object({
    type: z.enum(SUBMISSION_TYPES),
    title: z.string().trim().min(3).max(120),
    url: z.url().or(z.literal("")).optional(),
    text: z.string().trim().max(40_000).optional(),
    tags: z.array(z.string()).max(5).optional(),
  })
  .refine((v) => Boolean(v.url) !== Boolean(v.text), {
    message: "Provide a URL or text body, not both.",
  });

export type SubmissionInput = z.infer<typeof submissionInputSchema>;

export type SubmitResult =
  | { ok: true; submissionId: string; pending: boolean }
  | { ok: false; reason: "validation" | "locked" | "rate"; detail?: string }
  | { ok: false; reason: "duplicate"; existingId: string }
  | {
      // The AI policy moderator rejected the post. The row exists in
      // the DB with state='rejected' so an appeal can target it; the
      // caller's job is to surface the verdict to the user (not to
      // claim the publish succeeded). decisionId points at the
      // policy_decisions row used for /appeal/[id].
      //
      // decisionId is null only when the moderator rejected but the
      // post-insert audit-row write itself failed. The submission
      // is still rejected; users contact staff manually since
      // /appeal/[id] needs a real policy_decisions row.
      ok: false;
      reason: "rejected";
      submissionId: string;
      category: string;
      oneLineWhy: string;
      decisionId: string | null;
    };

/**
 * Entry-point provenance. Web traffic passes { surface: 'web' };
 * PAT-auth API/MCP traffic passes { surface: 'api', tokenId, tokenPrefix }.
 * For API submissions we write submitterKind='scout' and store the
 * full api_tokens.id UUID in sourceId so a submission can be
 * unambiguously traced back to one token (the 12-char displayPrefix
 * is not unique and would collide as token volume grows). tokenPrefix
 * is kept here only for human-readable log/error messages.
 */
export type SubmissionVia =
  | { surface: "web" }
  | { surface: "api"; tokenId: string; tokenPrefix: string };

export type DeleteSubmissionResult =
  | { ok: true }
  | { ok: false; reason: "not_found" | "forbidden" };

export const updateSubmissionInputSchema = z
  .object({
    title: z.string().trim().min(3).max(120).optional(),
    text: z.string().trim().max(40_000).optional(),
  })
  .refine((v) => v.title !== undefined || v.text !== undefined, {
    message: "Provide at least one of: title, text.",
  });

export type UpdateSubmissionInput = z.infer<typeof updateSubmissionInputSchema>;

export type UpdateSubmissionResult =
  | { ok: true; silent: boolean; updatedAt: Date | null }
  | {
      ok: false;
      reason: "not_found" | "forbidden" | "expired" | "noop" | "invalid";
      detail?: string;
    };
