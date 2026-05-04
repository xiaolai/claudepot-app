-- 0011_tag_admin_actions — extend moderation_action enum so the
-- tag-vocabulary admin surface (/admin/flags) can append to
-- moderation_log when staff create / rename / merge / retire tags.
--
-- Background: moderationAction in src/lib/actions/moderation.ts logs
-- every staff write through src/db/schema.ts:moderation_log.action.
-- Tag CRUD (src/lib/actions/admin-tag.ts) was added without a matching
-- enum value, so those writes had nowhere to land in the log — leaving
-- a gap in auditability (audit 2026-05-02, MEDIUM finding 1).
--
-- We extend the existing enum rather than introducing a parallel
-- tag_log table because the semantics are the same: "staff did a
-- governance action that other staff need to be able to review." The
-- enum stays the single discriminator on moderation_log.action.
--
-- ALTER TYPE … ADD VALUE IF NOT EXISTS is PG 9.6+. Idempotent; safe to
-- run twice. No backfill — existing rows retain their current values.

ALTER TYPE "public"."moderation_action" ADD VALUE IF NOT EXISTS 'tag_create';--> statement-breakpoint
ALTER TYPE "public"."moderation_action" ADD VALUE IF NOT EXISTS 'tag_rename';--> statement-breakpoint
ALTER TYPE "public"."moderation_action" ADD VALUE IF NOT EXISTS 'tag_merge';--> statement-breakpoint
ALTER TYPE "public"."moderation_action" ADD VALUE IF NOT EXISTS 'tag_retire';
