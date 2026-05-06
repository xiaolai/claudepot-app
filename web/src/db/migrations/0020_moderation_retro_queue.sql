-- 0020_moderation_retro_queue — comment fail-open backstop.
--
-- Per dev-docs/policy-moderator-plan.md §11, comment moderation must
-- fail OPEN on model error: publish optimistically, queue for
-- retroactive review. The plan called for this on day one but the
-- prior slice forced state='pending' instead because the queue
-- table didn't exist yet — Codex caught it as a partial fix.
--
-- This migration creates the queue. A cron job (web/src/app/api/cron/
-- moderation-retro/route.ts, scheduled in vercel.json) drains
-- pending entries by re-running moderate() against the freshly-
-- inserted row. If the retroactive verdict rejects, the comment
-- flips to state='rejected' via the same retract path the
-- confirmation pass uses; if it passes (or stays a model error),
-- the entry is marked done.
--
-- One row per (target_type, target_id, attempted_at). Repeated
-- retries on the same target are explicit history, not silent
-- replacements — calibration wants to see the trajectory.
--
-- 'queue_state' is its own enum so the cron's progress is queryable
-- without parsing free-text fields. 'pending' is the initial state;
-- the cron transitions to 'in_progress' under a SELECT … FOR UPDATE
-- SKIP LOCKED so concurrent cron invocations don't double-process,
-- then 'done' on success or 'failed' on terminal error.

CREATE TYPE "moderation_retro_state" AS ENUM (
  'pending',
  'in_progress',
  'done',
  'failed'
);
--> statement-breakpoint

CREATE TABLE IF NOT EXISTS "moderation_retro_queue" (
  "id"             uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  "target_type"    "target_type" NOT NULL,
  "target_id"      uuid NOT NULL,
  "author_id"      uuid NOT NULL REFERENCES "users"("id"),
  -- Snapshot of the original synthetic verdict so the cron can
  -- attribute the retro pass back to the failure that caused it
  -- (e.g. which prompt version was active, what error text).
  "trigger_reason" text NOT NULL,
  "state"          "moderation_retro_state" NOT NULL DEFAULT 'pending',
  "attempts"       smallint NOT NULL DEFAULT 0,
  "last_error"     text,
  "enqueued_at"    timestamptz NOT NULL DEFAULT now(),
  "started_at"     timestamptz,
  "completed_at"   timestamptz
);--> statement-breakpoint

-- Cron drain order: oldest pending first.
CREATE INDEX IF NOT EXISTS "idx_moderation_retro_queue_pending"
  ON "moderation_retro_queue" ("enqueued_at")
  WHERE "state" = 'pending';--> statement-breakpoint

-- Per-target lookup: one entry per attempt history; UI joins by
-- target to show recent retro passes alongside the moderator's
-- pass-1 / pass-2 entries on the audit page.
CREATE INDEX IF NOT EXISTS "idx_moderation_retro_queue_target"
  ON "moderation_retro_queue" ("target_type", "target_id", "enqueued_at" DESC);
