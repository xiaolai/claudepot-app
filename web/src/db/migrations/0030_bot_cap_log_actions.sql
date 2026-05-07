-- 0030_bot_cap_log_actions — log enum values for the bot-cap edits.
--
-- Mirrors the bot_exempt_grant / bot_exempt_revoke pattern from
-- migration 0019 so /admin/log shows cap changes alongside other
-- staff actions, filterable by the action discriminator without
-- parsing the free-text note.
--
-- Postgres 12+ permits ALTER TYPE ADD VALUE outside transactions
-- only when the new value isn't used in the same statement; both
-- values added here are not yet referenced by any constraint.

ALTER TYPE "moderation_action" ADD VALUE IF NOT EXISTS 'bot_cap_set';
--> statement-breakpoint
ALTER TYPE "moderation_action" ADD VALUE IF NOT EXISTS 'bot_cap_clear';
