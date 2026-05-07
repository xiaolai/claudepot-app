-- 0022_ai_tagging — Ada gains a second job: tag every accepted
-- submission with two tags during the same moderate() call.
--
-- Two columns added, both additive + idempotent:
--
-- 1. tags.pending_review — boolean, default false. When Ada
--    proposes a tag that doesn't exist in the vocabulary yet, the
--    new row goes in with pending_review=true and stays hidden
--    from the public /c catalog until staff approves it at
--    /admin/tags. Staff approval flips the flag to false; the tag
--    enters the live vocabulary.
--
-- 2. submission_tags.source — text, default 'user', constrained
--    to {'ai','user'}. Distinguishes Ada-applied tags from
--    user-applied tags (the latter come from the submit form's
--    tags field). Used for analytics ("did AI tags drive search
--    engagement?"), audit, and override logic (user-supplied
--    tags win on duplicates).
--
-- The existing 5-tag-per-submission cap (enforced in the input
-- schema) still holds: user can supply up to 5; Ada adds up to 2;
-- duplicates dedupe; the union is capped at 5 by the create path.

ALTER TABLE "tags"
  ADD COLUMN IF NOT EXISTS "pending_review" boolean NOT NULL DEFAULT false;
--> statement-breakpoint

ALTER TABLE "submission_tags"
  ADD COLUMN IF NOT EXISTS "source" text NOT NULL DEFAULT 'user';
--> statement-breakpoint

-- Postgres lacks `ADD CONSTRAINT IF NOT EXISTS`, so guard the add
-- via pg_catalog so a re-run (e.g. running this file by hand on a
-- DB where it already applied) doesn't error. The catalog query
-- looks for the constraint by exact name on the target table.
DO $$
BEGIN
  IF NOT EXISTS (
    SELECT 1 FROM pg_constraint
    WHERE conname = 'submission_tags_source_check'
      AND conrelid = 'submission_tags'::regclass
  ) THEN
    ALTER TABLE "submission_tags"
      ADD CONSTRAINT "submission_tags_source_check"
      CHECK ("source" IN ('ai', 'user'));
  END IF;
END$$;
--> statement-breakpoint

-- Index supports the /admin/tags review page (pending tags first,
-- then ordered by when Ada proposed them so newest goes to top).
-- No created_at column on tags today — adding one is a separate
-- slice; use slug as a tie-breaker for now.
CREATE INDEX IF NOT EXISTS "idx_tags_pending_review"
  ON "tags" ("pending_review", "slug")
  WHERE "pending_review" = true;
