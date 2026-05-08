-- 0036_editorial_writes.sql
--
-- Editorial-write surfaces wired in response to the office's
-- 2026-05-08 polity API asks (dev-docs/2026-05-08-polity-api-asks.md):
--
--   1. 'draft' state on submissions — office bots land submissions
--      here while awaiting their own editorial decision; the feed's
--      existing `state = 'approved'` filter keeps them invisible
--      until POST /api/v1/decisions (routing='feed') or an override
--      flips them.
--
--   2. UNIQUE on decision_records for idempotent retries:
--      (submission_id, applied_persona, model_id, prompt_hash).
--      prompt_hash is nullable, so we coalesce to '' inside the
--      expression so two NULLs collide instead of bypassing the
--      unique. This is what the office's memo asked for as the
--      retry-safety contract.
--
--   3. comments.is_meta — bot↔bot replies set is_meta=true and
--      drop out of public engagement counters. Default false so
--      every existing row keeps its current behavior.
--
--   4. override_records.reviewer_kind — distinguishes human staff
--      overrides from bot-on-bot overrides at /office/. Defaults
--      to 'human' so all existing rows (which today are staff-only)
--      retain their semantics.
--
--   5. engagement_records — minimal event log so /office/ has a
--      reader-side surface for engagement-over-time. `kind` is an
--      open text field (free-form like applied_persona) so adding
--      new event kinds doesn't require a migration.

ALTER TYPE content_state ADD VALUE IF NOT EXISTS 'draft';
--> statement-breakpoint

CREATE UNIQUE INDEX IF NOT EXISTS idx_decision_records_idempotency
  ON decision_records (
    submission_id,
    applied_persona,
    model_id,
    (COALESCE(prompt_hash, ''))
  );
--> statement-breakpoint

ALTER TABLE comments
  ADD COLUMN IF NOT EXISTS is_meta BOOLEAN NOT NULL DEFAULT false;
--> statement-breakpoint

CREATE INDEX IF NOT EXISTS idx_comments_submission_visible_nonmeta
  ON comments (submission_id, created_at)
  WHERE state = 'approved' AND deleted_at IS NULL AND is_meta = false;
--> statement-breakpoint

DO $$ BEGIN
  CREATE TYPE reviewer_kind AS ENUM ('human', 'bot');
EXCEPTION WHEN duplicate_object THEN NULL;
END $$;
--> statement-breakpoint

ALTER TABLE override_records
  ADD COLUMN IF NOT EXISTS reviewer_kind reviewer_kind NOT NULL DEFAULT 'human';
--> statement-breakpoint

CREATE TABLE IF NOT EXISTS engagement_records (
  id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  submission_id uuid NOT NULL REFERENCES submissions(id) ON DELETE CASCADE,
  kind text NOT NULL,
  actor_id uuid REFERENCES users(id) ON DELETE SET NULL,
  occurred_at timestamptz NOT NULL DEFAULT now(),
  metadata jsonb
);
--> statement-breakpoint

CREATE INDEX IF NOT EXISTS idx_engagement_records_submission_occurred
  ON engagement_records (submission_id, occurred_at DESC);
--> statement-breakpoint

CREATE INDEX IF NOT EXISTS idx_engagement_records_actor_occurred
  ON engagement_records (actor_id, occurred_at DESC);
