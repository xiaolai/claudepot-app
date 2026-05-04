-- Phase 11a: Postgres full-text search.
-- Generated tsvector column over title + text + url + tag slugs.
-- GIN index for fast lookup. Updated automatically as the row changes.

ALTER TABLE "submissions"
  ADD COLUMN IF NOT EXISTS "search_vec" tsvector
  GENERATED ALWAYS AS (
    to_tsvector(
      'english',
      coalesce(title, '') || ' ' ||
      coalesce(text, '') || ' ' ||
      coalesce(url, '')
    )
  ) STORED;
--> statement-breakpoint

CREATE INDEX IF NOT EXISTS "idx_submissions_search_vec"
  ON "submissions" USING GIN ("search_vec");
