/**
 * Zod schemas for the office's editorial write surface (migration
 * 0036). The route handlers under app/api/v1/decisions/**,
 * app/api/v1/scout-runs/** consume these; nothing else should
 * depend on them.
 *
 * `appliedPersona` and the per-criterion keys inside
 * `perCriterionScores` are deliberately open — the office owns the
 * vocabulary; the polity stores it as text/jsonb without
 * validation against any whitelist. New personas / criterion keys
 * can land in the office repo without a polity migration.
 */

import { z } from "zod";

import { SUBMISSION_TYPES } from "@/lib/submissions/schema";

/* ── Common building blocks ─────────────────────────────────── */

const isoTimestamp = z.iso.datetime({
  message: "must be an ISO-8601 timestamp",
});

// Reuse the canonical submission-type vocabulary so a new type
// added in lib/submissions/schema.ts (or migration 0008-style
// changes) doesn't silently break office decision writes.
const submissionTypeSchema = z.enum(SUBMISSION_TYPES);

const aiFinalDecisionSchema = z.enum([
  "accept",
  "reject",
  "borderline_to_human_queue",
]);

const routingDestinationSchema = z.enum(["feed", "firehose", "human_queue"]);

const confidenceBandSchema = z.enum(["high", "low"]);

/* ── Decision input ─────────────────────────────────────────── */

export const decisionInputSchema = z.object({
  submissionId: z.uuid(),
  rubricVersion: z.string().min(1).max(80),
  audienceDocVersion: z.string().min(1).max(80),
  appliedPersona: z.string().min(1).max(80),
  perCriterionScores: z.record(z.string(), z.number()),
  weightedTotal: z.number(),
  hardRejectsHit: z.array(z.string()).default([]),
  inclusionGates: z.record(z.string(), z.boolean()),
  typeInferred: submissionTypeSchema,
  subSegmentInferred: z.string().min(1).max(120),
  confidence: confidenceBandSchema,
  oneLineWhy: z.string().min(1).max(2000),
  finalDecision: aiFinalDecisionSchema,
  routing: routingDestinationSchema,
  modelId: z.string().min(1).max(120),
  promptHash: z.string().min(1).max(120).optional(),
  costUsd: z.number().nonnegative().max(1000).optional(),
});
export type DecisionInput = z.infer<typeof decisionInputSchema>;

/* ── Override input ─────────────────────────────────────────── */

export const overrideInputSchema = z.object({
  overrideDecision: aiFinalDecisionSchema,
  overrideRouting: routingDestinationSchema,
  reviewerScores: z.record(z.string(), z.number()).optional(),
  reason: z.string().min(1).max(2000),
});
export type OverrideInput = z.infer<typeof overrideInputSchema>;

/* ── Scout-run input ────────────────────────────────────────── */

export const scoutRunInputSchema = z
  .object({
    sourceId: z.string().min(1).max(120),
    startedAt: isoTimestamp,
    finishedAt: isoTimestamp,
    itemsPulled: z.number().int().nonnegative().max(1_000_000),
    itemsKept: z.number().int().nonnegative().max(1_000_000),
    itemsDropped: z.number().int().nonnegative().max(1_000_000),
    error: z.string().max(2000).optional(),
  })
  .refine(
    (v) => new Date(v.finishedAt).getTime() >= new Date(v.startedAt).getTime(),
    { message: "finishedAt must be ≥ startedAt", path: ["finishedAt"] },
  )
  .refine((v) => v.itemsKept + v.itemsDropped <= v.itemsPulled, {
    message: "itemsKept + itemsDropped cannot exceed itemsPulled",
    path: ["itemsKept"],
  });
export type ScoutRunInput = z.infer<typeof scoutRunInputSchema>;
