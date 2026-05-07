/**
 * Per-kind payload schemas for POST /api/v1/bots/reports.
 *
 * The `kind` discriminator is an open enum at the DB layer (text
 * column on bot_reports + bot_heartbeats UPSERT path), but every
 * accepted value lands here as a Zod schema. The route handler
 * rejects unknown kinds at the boundary so the table never carries
 * a payload schema we haven't audited.
 *
 * Adding a new kind:
 *   1. Add a payloadSchema entry below.
 *   2. Update KIND_SCHEMA_BY_KIND.
 *   3. Update the union in BotReportInput.
 *   4. The route + MCP tool pick it up automatically.
 */

import { z } from "zod";

/* ── Common building blocks ─────────────────────────────────── */

const isoTimestamp = z.iso.datetime({
  message: "must be an ISO-8601 timestamp",
});

const usdAmount = z
  .number()
  .nonnegative({ message: "USD amount must be ≥ 0" })
  .max(1_000_000, { message: "USD amount exceeds sanity cap" });

/* ── Heartbeat ──────────────────────────────────────────────── */

export const heartbeatPayloadSchema = z.object({
  /** Bot's self-reported build identifier (commit sha, semver, etc). */
  version: z.string().min(1).max(120).optional(),
  /** 'prod' | 'staging' | 'dev' | bot-defined. */
  env: z.string().min(1).max(40).optional(),
  /** Free-form bot-defined metadata; kept small so the row stays cheap. */
  meta: z.record(z.string(), z.unknown()).optional(),
});

/* ── Work summary ───────────────────────────────────────────── */

export const workSummaryPayloadSchema = z.object({
  windowStart: isoTimestamp,
  windowEnd: isoTimestamp,
  /** Free-shape unit counter map: { posts_classified: 12, tags_applied: 7, ... }. */
  units: z.record(z.string(), z.number().int().nonnegative()),
  notes: z.string().max(2000).optional(),
});

/* ── Cost ───────────────────────────────────────────────────── */

export const costPayloadSchema = z.object({
  provider: z.string().min(1).max(40),
  model: z.string().min(1).max(120),
  inputTokens: z.number().int().nonnegative().optional(),
  outputTokens: z.number().int().nonnegative().optional(),
  usd: usdAmount,
  notes: z.string().max(500).optional(),
});

/* ── Error ──────────────────────────────────────────────────── */

export const errorPayloadSchema = z.object({
  severity: z.enum(["warn", "error"]),
  message: z.string().min(1).max(2000),
  context: z.record(z.string(), z.unknown()).optional(),
});

/* ── Proposal ───────────────────────────────────────────────── */

/**
 * The bot wants a human to ack a change. Surfaces in the /admin
 * Today inbox NoticeStrip. Sub-kind enumerated to keep operator
 * mental model simple — adding more is one line + one icon.
 */
export const proposalPayloadSchema = z.object({
  /** Sub-kind: what the bot wants to change. Closed enum here. */
  kind: z.enum([
    "vocab_tag",
    "block_user",
    "tag_merge",
    "tag_retire",
    "general",
  ]),
  /** Free-form. The notice strip renders this verbatim. */
  reason: z.string().min(1).max(2000),
  /** Optional pointer to a target row (submission/comment/user uuid, or a tag slug). */
  target: z.string().min(1).max(200).optional(),
  /**
   * Dedup key. While the proposal is `status='open'`, repeated
   * posts with the same (bot_id, key) are rejected by a partial
   * unique index. Bots that don't supply a key get free
   * duplicates — fine for one-shot proposals, awkward for retried
   * ones. Recommend setting it.
   */
  key: z.string().min(1).max(200).optional(),
});

/* ── Decision summary ───────────────────────────────────────── */

/**
 * Moderation-class bots self-report verdict distribution + drift
 * z-score over a window. Lets the operator see "Ada's reject rate
 * jumped 3 sigma since the prompt change" without manually
 * querying policy_decisions.
 */
export const decisionSummaryPayloadSchema = z.object({
  windowStart: isoTimestamp,
  windowEnd: isoTimestamp,
  /** Verdict counts: { pass: 142, reject: 8 } or richer. */
  verdicts: z.record(z.string(), z.number().int().nonnegative()),
  /** Confidence histogram: { high: 130, low: 20 }. */
  confidence: z.record(z.string(), z.number().int().nonnegative()).optional(),
  /** z-score vs the bot's own baseline. >|2| is operator-worthy. */
  driftZ: z.number().optional(),
  notes: z.string().max(2000).optional(),
});

/* ── Discriminated union + dispatch table ───────────────────── */

export const REPORT_KINDS = [
  "heartbeat",
  "work_summary",
  "cost",
  "error",
  "proposal",
  "decision_summary",
] as const;

export type ReportKind = (typeof REPORT_KINDS)[number];

export const KIND_SCHEMA_BY_KIND: Record<ReportKind, z.ZodTypeAny> = {
  heartbeat: heartbeatPayloadSchema,
  work_summary: workSummaryPayloadSchema,
  cost: costPayloadSchema,
  error: errorPayloadSchema,
  proposal: proposalPayloadSchema,
  decision_summary: decisionSummaryPayloadSchema,
};

export function isReportKind(s: string): s is ReportKind {
  return (REPORT_KINDS as readonly string[]).includes(s);
}

/**
 * Top-level request body. The route handler does a two-step parse:
 * first extract `kind` and route to the per-kind schema; then
 * re-run the discriminated parse so payload-shape errors point at
 * the field, not at the union.
 */
export const reportInputSchema = z.object({
  kind: z.enum(REPORT_KINDS),
  payload: z.record(z.string(), z.unknown()),
  /**
   * Optional explicit cost override. If omitted on a `cost` kind,
   * the route derives cost_usd from payload.usd. Useful for
   * bots that want to attribute non-cost reports to a billing
   * pool (rare).
   */
  costUsd: usdAmount.optional(),
});

export type ReportInput = z.infer<typeof reportInputSchema>;

/* ── Display labels for UI ──────────────────────────────────── */

export const KIND_LABELS: Record<ReportKind, string> = {
  heartbeat: "heartbeat",
  work_summary: "work",
  cost: "cost",
  error: "error",
  proposal: "proposal",
  decision_summary: "decision summary",
};
