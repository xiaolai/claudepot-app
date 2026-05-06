-- 0021_moderation_prompts — DB-backed system prompt for the AI
-- policy moderator, editable via /admin/policy-prompt.
--
-- Why a table at all when the prompt was a TypeScript constant
-- through Slice 1a/1b/2/3: calibrating false-positive rates is
-- iterative, and bouncing each tweak through a redeploy cycle
-- breaks flow. The editor lets staff (Ada is the persona; the
-- human staff member acting via /admin) save a new version and
-- activate it without a deploy.
--
-- Constraints enforced at the DB level:
--
--   1. UNIQUE on `version` — each version label is unique.
--   2. Partial unique index on (active=true) — at most one active
--      version at a time. Activation flips happen in a single
--      transaction (UPDATE … SET active=false WHERE active=true;
--      INSERT new row WITH active=true) so the index never sees
--      a transient state with two actives.
--
-- Fallback: when the table is empty (e.g. fresh deploy where no
-- staff has yet saved a version), the moderator's prompt-store
-- returns the hardcoded constants from web/src/lib/moderation/prompt.ts
-- and stamps policy_decisions.prompt_version = 'fallback'. So the
-- moderator works correctly the moment migration 0021 runs, even
-- before /admin/policy-prompt is ever visited.

CREATE TABLE IF NOT EXISTS "moderation_prompts" (
  "id"            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  "version"       text NOT NULL UNIQUE,
  "system_prompt" text NOT NULL,
  "active"        boolean NOT NULL DEFAULT false,
  "created_by"    uuid NOT NULL REFERENCES "users"("id"),
  "created_at"    timestamptz NOT NULL DEFAULT now(),
  "note"          text
);--> statement-breakpoint

CREATE UNIQUE INDEX IF NOT EXISTS "idx_moderation_prompts_active"
  ON "moderation_prompts" ("active")
  WHERE "active" = true;--> statement-breakpoint

CREATE INDEX IF NOT EXISTS "idx_moderation_prompts_created"
  ON "moderation_prompts" ("created_at" DESC);
