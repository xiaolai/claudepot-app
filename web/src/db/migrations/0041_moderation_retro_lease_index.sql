-- 0041_moderation_retro_lease_index — make stale lease reclamation cheap.
-- The worker now reclaims rows stranded in `in_progress` after a
-- serverless process disappears. This partial index keeps that recovery
-- query bounded without enlarging the hot pending queue index.

CREATE INDEX IF NOT EXISTS "idx_moderation_retro_queue_in_progress"
  ON "moderation_retro_queue" ("started_at")
  WHERE "state" = 'in_progress';
