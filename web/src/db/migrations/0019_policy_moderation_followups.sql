-- 0019_policy_moderation_followups — close the audit follow-ups.
--
-- Three changes:
--
-- 1. target_type_enum gains 'user'. Slice 3's rung-4 ban-candidate
--    flags previously had to attach to a parent submission because
--    the enum didn't allow user-level targets, which mis-stated the
--    evidence and pointed /admin/queue's destructive actions at the
--    wrong row. With 'user' as a target, ban-candidate flags now
--    point at the user under review directly.
--
-- 2. moderation_action_enum gains 'bot_exempt_grant' and
--    'bot_exempt_revoke'. Previously /admin/users toggle reused
--    'approve' for both grant and revoke, distinguishing only via
--    the note prefix. The enum extension lets /admin/log filter on
--    the action discriminator without parsing free-text notes.
--
-- 3. Partial unique index on flags(target_type, target_id) WHERE
--    status='open' AND reason LIKE 'appeal:%' enforces the
--    one-open-appeal-per-target invariant at the DB level. App-side
--    SELECT-then-INSERT was racy; the unique index makes concurrent
--    duplicate appeals fail loudly via constraint violation, which
--    the appeal core catches and translates to reason='duplicate'.

ALTER TYPE "target_type" ADD VALUE IF NOT EXISTS 'user';
--> statement-breakpoint

ALTER TYPE "moderation_action" ADD VALUE IF NOT EXISTS 'bot_exempt_grant';
--> statement-breakpoint
ALTER TYPE "moderation_action" ADD VALUE IF NOT EXISTS 'bot_exempt_revoke';
--> statement-breakpoint

CREATE UNIQUE INDEX IF NOT EXISTS "idx_flags_open_appeal_per_target"
  ON "flags" ("target_type", "target_id")
  WHERE "status" = 'open' AND "reason" LIKE 'appeal:%';
