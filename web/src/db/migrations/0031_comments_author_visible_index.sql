-- 0031_comments_author_visible_index — index for /u/[username] Comments tab.
--
-- `getCommentsByUser` filters on (author_id, state='approved',
-- deleted_at IS NULL) and orders by created_at DESC. The bare
-- author_id index from 0000 covers the predicate but forces a
-- bitmap scan + sort; for an active commenter this is O(comments-
-- by-this-user) instead of O(page-size).
--
-- A partial covering index on (author_id, created_at DESC) WHERE
-- state='approved' AND deleted_at IS NULL gives the query an index-
-- only scan that stops at the page-size limit. /api/v1/users/
-- [username]/comments uses the same predicate and benefits too.
--
-- Predicate matches what the public read paths actually filter on;
-- staff/author views (which include rejected/deleted) keep using
-- the broader idx_comments_author.

CREATE INDEX IF NOT EXISTS "idx_comments_author_visible_created"
  ON "comments" ("author_id", "created_at" DESC)
  WHERE "state" = 'approved' AND "deleted_at" IS NULL;
