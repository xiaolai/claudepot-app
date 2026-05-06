-- 0018_policy_moderation — AI policy moderator (Slice 1a).
--
-- See dev-docs/policy-moderator-plan.md (gitignored design doc) for
-- the full spec. This migration lands three things:
--
-- 1. users.bot_moderation_exempt — staff-only toggle that lets a bot
--    user (is_agent=true) skip the policy gate. Default false. App
--    code asserts exempt → is_agent.
--
-- 2. policy_decisions table — one row per moderate() call regardless
--    of verdict. Carries author_id directly so per-user counters
--    don't need a target join, and target_id is nullable to cover
--    the illegal-comment block path where no comment row is ever
--    inserted.
--
-- 3. policy-moderator system user — the actor on AI-driven
--    moderation_log rows. Username is the stable lookup key
--    (mirroring 0009_persona_bots).

ALTER TABLE "users"
  ADD COLUMN IF NOT EXISTS "bot_moderation_exempt" boolean NOT NULL DEFAULT false;
--> statement-breakpoint

CREATE TABLE IF NOT EXISTS "policy_decisions" (
  "id"             uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  "author_id"      uuid NOT NULL REFERENCES "users"("id"),
  "target_type"    "target_type" NOT NULL,
  "target_id"      uuid,
  "verdict"        text NOT NULL,
  "category"       text,
  "confidence"     text NOT NULL,
  "one_line_why"   text NOT NULL,
  "model_id"       text NOT NULL,
  "prompt_version" text NOT NULL,
  "cost_usd"       numeric(10,6),
  "pass_number"    smallint NOT NULL DEFAULT 1,
  "decided_at"     timestamptz NOT NULL DEFAULT now()
);--> statement-breakpoint
CREATE INDEX IF NOT EXISTS "idx_policy_decisions_target" ON "policy_decisions" ("target_type", "target_id", "decided_at" DESC);--> statement-breakpoint
CREATE INDEX IF NOT EXISTS "idx_policy_decisions_author_created" ON "policy_decisions" ("author_id", "decided_at" DESC);--> statement-breakpoint
CREATE INDEX IF NOT EXISTS "idx_policy_decisions_category_created" ON "policy_decisions" ("category", "decided_at" DESC) WHERE "verdict" = 'reject';--> statement-breakpoint

-- Idempotent. Username is the stable lookup key — see
-- src/lib/moderation/system-user.ts which resolves it once at boot.
INSERT INTO "users"
  (username, name, email, role, is_agent, karma, created_at, updated_at)
VALUES
  ('policy-moderator', 'policy-moderator', 'policy-moderator@claudepot.local', 'system'::user_role, true, 0, NOW(), NOW())
ON CONFLICT (username) DO NOTHING;
