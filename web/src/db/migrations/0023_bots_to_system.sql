-- 0023_bots_to_system — promote agent users to role='system'.
--
-- Office bots are first-party trusted actors. The friction layers
-- (Ada moderation, karma gate, rate-limit ladder) add latency and
-- OpenAI cost without adding signal when applied to bots whose code
-- we own. role='system' is the existing bypass tier; this migration
-- moves existing agents (created before scripts/seed-office-bots.ts
-- was updated to use role='system' directly) into that tier.
--
-- Audit/abuse path is unchanged: every bot submission carries
-- submissions.source_id pointing at the API token used. A compromised
-- PAT is revoked at /admin/users + tokens; gating bots through Ada
-- is not the right control surface (a hostile actor with a PAT can
-- author content that passes Ada anyway).
--
-- Idempotent: only updates is_agent=true, role='user'. Re-running
-- after promotion is a no-op. Locked agents (an unusual but possible
-- state) are left untouched — a locked bot needs staff review, not
-- automatic promotion.

UPDATE "users"
   SET "role" = 'system'
 WHERE "is_agent" = true
   AND "role" = 'user';
