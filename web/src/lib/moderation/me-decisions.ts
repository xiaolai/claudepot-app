/**
 * Shared core for "list my own AI moderation decisions".
 *
 * Used by both /api/v1/me/decisions (REST) and the list_my_decisions
 * MCP tool. Returns at most HARD_LIMIT rows for the given user,
 * filtered by kind + since. No cursor — see route.ts for rationale.
 */

import { z } from "zod";
import { and, desc, eq, gt } from "drizzle-orm";

import { db } from "@/db/client";
import { policyDecisions } from "@/db/schema";

export const HARD_LIMIT = 200;

export const listMyDecisionsInputSchema = z.object({
  kind: z.enum(["submission", "comment"]).optional(),
  since: z.iso.datetime().optional(),
});

export type ListMyDecisionsInput = z.infer<typeof listMyDecisionsInputSchema>;

export interface PolicyDecisionDto {
  id: string;
  // Includes 'user' as of migration 0019 — surfaces here for
  // completeness even though policy_decisions itself only inserts
  // 'submission' or 'comment' today. Future user-targeted decisions
  // (e.g. account-level flags) can land without a DTO change.
  targetType: "submission" | "comment" | "user";
  targetId: string | null;
  verdict: string;
  category: string | null;
  confidence: string;
  oneLineWhy: string;
  modelId: string;
  promptVersion: string;
  passNumber: number;
  decidedAt: string;
}

export async function listMyDecisions(
  userId: string,
  input: ListMyDecisionsInput,
): Promise<{ items: PolicyDecisionDto[] }> {
  const conditions = [eq(policyDecisions.authorId, userId)];
  if (input.kind) {
    conditions.push(eq(policyDecisions.targetType, input.kind));
  }
  if (input.since) {
    conditions.push(gt(policyDecisions.decidedAt, new Date(input.since)));
  }

  const rows = await db
    .select({
      id: policyDecisions.id,
      targetType: policyDecisions.targetType,
      targetId: policyDecisions.targetId,
      verdict: policyDecisions.verdict,
      category: policyDecisions.category,
      confidence: policyDecisions.confidence,
      oneLineWhy: policyDecisions.oneLineWhy,
      modelId: policyDecisions.modelId,
      promptVersion: policyDecisions.promptVersion,
      passNumber: policyDecisions.passNumber,
      decidedAt: policyDecisions.decidedAt,
    })
    .from(policyDecisions)
    .where(and(...conditions))
    .orderBy(desc(policyDecisions.decidedAt))
    .limit(HARD_LIMIT);

  return {
    items: rows.map((r) => ({
      id: r.id,
      targetType: r.targetType,
      targetId: r.targetId,
      verdict: r.verdict,
      category: r.category,
      confidence: r.confidence,
      oneLineWhy: r.oneLineWhy,
      modelId: r.modelId,
      promptVersion: r.promptVersion,
      passNumber: r.passNumber,
      decidedAt: r.decidedAt.toISOString(),
    })),
  };
}
