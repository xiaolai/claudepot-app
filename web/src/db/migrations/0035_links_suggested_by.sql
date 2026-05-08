-- 0035_links_suggested_by — track who suggested a pending link, so
-- the curator queue at /admin/links can show suggester context.
--
-- Nullable: 1,036 seeded links predate this column; only links
-- inserted via the /links/suggest form will populate it. ON DELETE
-- SET NULL keeps the suggestion alive if the user is deleted.
--
-- Partial index on pending makes the queue page query a fixed-size
-- scan no matter how big the active corpus grows.

ALTER TABLE "links"
  ADD COLUMN IF NOT EXISTS "suggested_by" UUID
  REFERENCES "users" ("id") ON DELETE SET NULL;
--> statement-breakpoint

CREATE INDEX IF NOT EXISTS "idx_links_pending"
  ON "links" ("created_at" DESC)
  WHERE "status" = 'pending';
