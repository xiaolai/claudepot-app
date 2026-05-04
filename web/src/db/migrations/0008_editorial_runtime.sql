-- 0008_editorial_runtime — replaces v1 ai_decisions / moderation_overrides
-- scaffolding with the per-criterion / per-persona decision model from
-- editorial/rubric.yml v0.2.3, plus override_records and scout_runs that
-- match editorial/audits/README.md substrates.
--
-- Bot-side writers live in the shannon-office private repo on
-- mac-mini-home; reader-side readers live in this repo. See
-- editorial/transparency.md for the privacy split.
--
-- The v1 ai_decisions / moderation_overrides tables had no application
-- code referencing them (per src/db/schema.ts header — "reserved for
-- v2-of-v2; unwritten in v1"). Safe to drop.

-- 1. New enums for the editorial runtime.
CREATE TYPE "submitter_kind" AS ENUM ('user', 'scout', 'import');--> statement-breakpoint

CREATE TYPE "ai_final_decision" AS ENUM ('accept', 'reject', 'borderline_to_human_queue');--> statement-breakpoint

CREATE TYPE "routing_destination" AS ENUM ('feed', 'firehose', 'human_queue');--> statement-breakpoint

CREATE TYPE "confidence_band" AS ENUM ('high', 'low');--> statement-breakpoint

-- 2. Extend submission_type with rubric v0.2.3 types. Postgres requires
--    one ALTER per value; IF NOT EXISTS guards re-runs.
ALTER TYPE "submission_type" ADD VALUE IF NOT EXISTS 'release';--> statement-breakpoint
ALTER TYPE "submission_type" ADD VALUE IF NOT EXISTS 'paper';--> statement-breakpoint
ALTER TYPE "submission_type" ADD VALUE IF NOT EXISTS 'workflow';--> statement-breakpoint
ALTER TYPE "submission_type" ADD VALUE IF NOT EXISTS 'case_study';--> statement-breakpoint
ALTER TYPE "submission_type" ADD VALUE IF NOT EXISTS 'prompt_pattern';--> statement-breakpoint

-- 3. Extend submissions with submitter provenance.
ALTER TABLE "submissions" ADD COLUMN IF NOT EXISTS "submitter_kind" "submitter_kind" NOT NULL DEFAULT 'user';--> statement-breakpoint
ALTER TABLE "submissions" ADD COLUMN IF NOT EXISTS "source_id" text;--> statement-breakpoint
CREATE INDEX IF NOT EXISTS "idx_submissions_source" ON "submissions" ("source_id");--> statement-breakpoint

-- 4. Drop v1 scaffolding (no consumers — verified safe).
DROP TABLE IF EXISTS "moderation_overrides";--> statement-breakpoint
DROP TABLE IF EXISTS "ai_decisions";--> statement-breakpoint
DROP TYPE IF EXISTS "ai_decision";--> statement-breakpoint

-- 5. decision_records — one row per agent scoring decision, matching
--    the rubric.yml v0.2.3 decision_record schema.
CREATE TABLE "decision_records" (
  "id"                    uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  "submission_id"         uuid NOT NULL REFERENCES "submissions"("id") ON DELETE CASCADE,
  "rubric_version"        text NOT NULL,
  "audience_doc_version"  text NOT NULL,
  "applied_persona"       text NOT NULL,
  "per_criterion_scores"  jsonb NOT NULL,
  "weighted_total"        numeric(8,3) NOT NULL,
  "hard_rejects_hit"      jsonb NOT NULL DEFAULT '[]'::jsonb,
  "inclusion_gates"       jsonb NOT NULL,
  "type_inferred"         "submission_type" NOT NULL,
  "sub_segment_inferred"  text NOT NULL,
  "confidence"            "confidence_band" NOT NULL,
  "one_line_why"          text NOT NULL,
  "final_decision"        "ai_final_decision" NOT NULL,
  "routing"               "routing_destination" NOT NULL,
  "model_id"              text NOT NULL,
  "prompt_hash"           text,
  "cost_usd"              numeric(10,6),
  "scored_at"             timestamptz NOT NULL DEFAULT now()
);--> statement-breakpoint
CREATE INDEX "idx_decision_records_submission" ON "decision_records" ("submission_id");--> statement-breakpoint
CREATE INDEX "idx_decision_records_routing" ON "decision_records" ("routing", "scored_at" DESC);--> statement-breakpoint
CREATE INDEX "idx_decision_records_persona" ON "decision_records" ("applied_persona", "scored_at" DESC);--> statement-breakpoint
CREATE INDEX "idx_decision_records_rubric_version" ON "decision_records" ("rubric_version");--> statement-breakpoint

-- 6. override_records — one row per human override of a decision_record.
CREATE TABLE "override_records" (
  "id"                  uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  "decision_record_id"  uuid NOT NULL REFERENCES "decision_records"("id") ON DELETE CASCADE,
  "reviewer_id"         uuid NOT NULL REFERENCES "users"("id"),
  "original_decision"   "ai_final_decision" NOT NULL,
  "override_decision"   "ai_final_decision" NOT NULL,
  "override_routing"    "routing_destination" NOT NULL,
  "reviewer_scores"     jsonb,
  "reason"              text NOT NULL,
  "created_at"          timestamptz NOT NULL DEFAULT now()
);--> statement-breakpoint
CREATE INDEX "idx_override_records_decision" ON "override_records" ("decision_record_id");--> statement-breakpoint
CREATE INDEX "idx_override_records_reviewer" ON "override_records" ("reviewer_id", "created_at" DESC);--> statement-breakpoint
CREATE INDEX "idx_override_records_created" ON "override_records" ("created_at" DESC);--> statement-breakpoint

-- 7. scout_runs — one row per scout invocation per source. Aggregated
--    counts surface on /office/sources; per-source rules stay private
--    inside shannon-office.
CREATE TABLE "scout_runs" (
  "id"            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  "source_id"     text NOT NULL,
  "started_at"    timestamptz NOT NULL,
  "finished_at"   timestamptz NOT NULL,
  "items_pulled"  integer NOT NULL DEFAULT 0,
  "items_kept"    integer NOT NULL DEFAULT 0,
  "items_dropped" integer NOT NULL DEFAULT 0,
  "error"         text
);--> statement-breakpoint
CREATE INDEX "idx_scout_runs_source_started" ON "scout_runs" ("source_id", "started_at" DESC);--> statement-breakpoint
CREATE INDEX "idx_scout_runs_started" ON "scout_runs" ("started_at" DESC);
