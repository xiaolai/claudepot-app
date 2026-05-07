-- 0026_user_tag_follows — let signed-in users follow a tag from /c/[slug].
--
-- Mirror of user_tag_mutes (0008): one row per (user_id, tag_slug),
-- composite PK so flipping is idempotent. ON DELETE CASCADE on both
-- foreign keys so a user-delete or tag-delete cleans up the joins
-- without leaving orphans.
--
-- Followed tags are read by /saved-style "your followed tags" surfaces
-- and any future personalized-feed work; this migration only creates
-- the table — the read paths land in subsequent feature commits.
--
-- No data backfill: the table starts empty. Existing tag-mutes are
-- not auto-converted (mute and follow are independent: a user can
-- mute a tag they haven't followed, follow a tag they haven't muted,
-- or — pathologically — both at once; the read paths decide policy).

CREATE TABLE "user_tag_follows" (
  "user_id"     uuid NOT NULL REFERENCES "users"("id") ON DELETE CASCADE,
  "tag_slug"    text NOT NULL REFERENCES "tags"("slug") ON DELETE CASCADE,
  "followed_at" timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY ("user_id", "tag_slug")
);--> statement-breakpoint

CREATE INDEX "idx_user_tag_follows_user" ON "user_tag_follows" ("user_id", "followed_at" DESC);--> statement-breakpoint
CREATE INDEX "idx_user_tag_follows_tag" ON "user_tag_follows" ("tag_slug");
