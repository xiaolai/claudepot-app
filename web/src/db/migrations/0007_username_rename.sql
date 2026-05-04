-- Replaces the forced-onboarding gate (0006_username_set) with the
-- auto-assign + grace-rename pattern. New OAuth signups get a clean
-- unique username immediately (see assignUsername in src/lib/username-
-- assign.ts) and may rename themselves up to MAX_SELF_RENAMES times
-- inside SELF_RENAME_GRACE_DAYS, with SELF_RENAME_COOLDOWN_MINUTES
-- between renames. After the grace window or count is exhausted, only
-- admins can change the username.

ALTER TABLE "users" DROP COLUMN IF EXISTS "username_set";--> statement-breakpoint
ALTER TABLE "users" ADD COLUMN IF NOT EXISTS "username_last_changed_at" timestamptz;--> statement-breakpoint
ALTER TABLE "users" ADD COLUMN IF NOT EXISTS "self_username_rename_count" integer NOT NULL DEFAULT 0;
