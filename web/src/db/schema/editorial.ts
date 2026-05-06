/**
 * Editorial runtime tables (added in 0008_editorial_runtime).
 *
 * Replaces the v1 ai_decisions / moderation_overrides scaffolding
 * (no consumers) with the richer per-criterion / per-persona shape
 * that matches `editorial/rubric.yml` v0.2.3.
 *
 * Bot-side writers live in the `claudepot-office` private repo on
 * mac-mini-home; reader-side readers live in this repo. See
 * `editorial/transparency.md` for the privacy split.
 *
 * NOT to be confused with policy_decisions (this repo, src/lib/moderation/) —
 * editorial = taste; policy = abuse/spam/etc.
 */

import {
  index,
  integer,
  jsonb,
  numeric,
  pgTable,
  text,
  timestamp,
  uuid,
} from "drizzle-orm/pg-core";

import {
  aiFinalDecisionEnum,
  confidenceBandEnum,
  routingDestinationEnum,
  submissionTypeEnum,
} from "./enums";
import { submissions } from "./content";
import { users } from "./users";

export const decisionRecords = pgTable(
  "decision_records",
  {
    id: uuid("id").primaryKey().defaultRandom(),
    submissionId: uuid("submission_id")
      .notNull()
      .references(() => submissions.id, { onDelete: "cascade" }),
    rubricVersion: text("rubric_version").notNull(),
    audienceDocVersion: text("audience_doc_version").notNull(),
    appliedPersona: text("applied_persona").notNull(), // open enum — new personas land without migration
    perCriterionScores: jsonb("per_criterion_scores").notNull(),
    weightedTotal: numeric("weighted_total", { precision: 8, scale: 3 }).notNull(),
    hardRejectsHit: jsonb("hard_rejects_hit").notNull().default([]),
    inclusionGates: jsonb("inclusion_gates").notNull(),
    typeInferred: submissionTypeEnum("type_inferred").notNull(),
    subSegmentInferred: text("sub_segment_inferred").notNull(),
    confidence: confidenceBandEnum("confidence").notNull(),
    oneLineWhy: text("one_line_why").notNull(),
    finalDecision: aiFinalDecisionEnum("final_decision").notNull(),
    routing: routingDestinationEnum("routing").notNull(),
    modelId: text("model_id").notNull(),
    promptHash: text("prompt_hash"),
    costUsd: numeric("cost_usd", { precision: 10, scale: 6 }),
    scoredAt: timestamp("scored_at", { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [
    index("idx_decision_records_submission").on(t.submissionId),
    index("idx_decision_records_routing").on(t.routing, t.scoredAt.desc()),
    index("idx_decision_records_persona").on(t.appliedPersona, t.scoredAt.desc()),
    index("idx_decision_records_rubric_version").on(t.rubricVersion),
  ],
);

export const overrideRecords = pgTable(
  "override_records",
  {
    id: uuid("id").primaryKey().defaultRandom(),
    decisionRecordId: uuid("decision_record_id")
      .notNull()
      .references(() => decisionRecords.id, { onDelete: "cascade" }),
    reviewerId: uuid("reviewer_id")
      .notNull()
      .references(() => users.id),
    originalDecision: aiFinalDecisionEnum("original_decision").notNull(),
    overrideDecision: aiFinalDecisionEnum("override_decision").notNull(),
    overrideRouting: routingDestinationEnum("override_routing").notNull(),
    reviewerScores: jsonb("reviewer_scores"), // optional per-criterion re-score
    reason: text("reason").notNull(),
    createdAt: timestamp("created_at", { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [
    index("idx_override_records_decision").on(t.decisionRecordId),
    index("idx_override_records_reviewer").on(t.reviewerId, t.createdAt.desc()),
    index("idx_override_records_created").on(t.createdAt.desc()),
  ],
);

/**
 * scout_runs — one row per scout invocation per source. Aggregated
 * counts surface on /office/sources; per-source rules stay private
 * inside the claudepot-office repo.
 */
export const scoutRuns = pgTable(
  "scout_runs",
  {
    id: uuid("id").primaryKey().defaultRandom(),
    sourceId: text("source_id").notNull(), // matches editorial sources.yml id
    startedAt: timestamp("started_at", { withTimezone: true }).notNull(),
    finishedAt: timestamp("finished_at", { withTimezone: true }).notNull(),
    itemsPulled: integer("items_pulled").notNull().default(0),
    itemsKept: integer("items_kept").notNull().default(0),
    itemsDropped: integer("items_dropped").notNull().default(0),
    error: text("error"),
  },
  (t) => [
    index("idx_scout_runs_source_started").on(t.sourceId, t.startedAt.desc()),
    index("idx_scout_runs_started").on(t.startedAt.desc()),
  ],
);
