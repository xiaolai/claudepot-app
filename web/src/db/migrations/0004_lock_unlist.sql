-- Audit finding 3.3: `lock` and `unlist` moderation actions previously
-- logged but didn't change behavior. Add the columns the actions need.
--   locked_at — set by moderationAction(lock); blocks new comments via
--               submitComment. Cleared by `restore`.
--   unlisted_at — set by moderationAction(unlist); excludes from feeds
--                 but the permalink (/post/[id]) still resolves.
--                 Cleared by `restore`.

ALTER TABLE "submissions" ADD COLUMN IF NOT EXISTS "locked_at" timestamptz;--> statement-breakpoint
ALTER TABLE "submissions" ADD COLUMN IF NOT EXISTS "unlisted_at" timestamptz;
