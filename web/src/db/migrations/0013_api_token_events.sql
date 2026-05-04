-- 0013_api_token_events — audit trail for PAT lifecycle events.
--
-- Records every mint, revoke, and (later) auto-expire / rate-limit-trip
-- against api_tokens. The moderation_log table was an awkward fit:
-- moderation_log records *staff* actions, while PAT mints/revokes are
-- typically *user* actions on their own tokens. Keeping the surfaces
-- separate avoids overloading the moderation_action enum and lets the
-- two logs evolve independently.
--
-- token_id uses ON DELETE SET NULL so the audit row outlives the token
-- row (you usually want to know "who minted that token we can no longer
-- find?"). user_id uses ON DELETE CASCADE to follow the user-deletion
-- anonymization path used elsewhere in the schema.
--
-- The event enum is closed so misspellings can't silently slip past
-- compliance queries; new event types require an explicit ALTER TYPE.

CREATE TYPE "api_token_event" AS ENUM ('mint', 'revoke');
--> statement-breakpoint

CREATE TABLE "api_token_events" (
  "id"          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  "token_id"    uuid REFERENCES "api_tokens"("id") ON DELETE SET NULL,
  "user_id"     uuid NOT NULL REFERENCES "users"("id") ON DELETE CASCADE,
  "event"       "api_token_event" NOT NULL,
  "scopes"      text[],
  "metadata"    jsonb,
  "occurred_at" timestamptz NOT NULL DEFAULT now()
);
--> statement-breakpoint

CREATE INDEX "idx_api_token_events_user"
  ON "api_token_events" ("user_id", "occurred_at" DESC);
--> statement-breakpoint

CREATE INDEX "idx_api_token_events_token"
  ON "api_token_events" ("token_id", "occurred_at" DESC);
