-- Phase 3: Auth.js v5 + DrizzleAdapter requires specific column names
-- on the users table (`name`, `image`). Our app uses `username` and
-- `avatar_url` semantically. Add the Auth.js columns alongside;
-- on OAuth signup, the adapter writes `name`/`image` and we mirror
-- them into `username`/`avatar_url` from app code.

ALTER TABLE "users" ADD COLUMN IF NOT EXISTS "name" text;--> statement-breakpoint
ALTER TABLE "users" ADD COLUMN IF NOT EXISTS "image" text;--> statement-breakpoint

-- Backfill: existing seeded users get name=username, image=avatar_url.
UPDATE "users" SET "name" = "username" WHERE "name" IS NULL;--> statement-breakpoint
UPDATE "users" SET "image" = "avatar_url" WHERE "image" IS NULL AND "avatar_url" IS NOT NULL;
