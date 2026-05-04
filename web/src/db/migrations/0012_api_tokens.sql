-- 0012_api_tokens — public API surface foundation.
--
-- Adds two tables that back the public REST + MCP API:
--
--   api_tokens         per-user Personal Access Tokens. Plaintext
--                      ("shn_pat_<28 url-safe-base64>") shown once at
--                      creation; only the SHA-256 hex digest is stored.
--                      `scopes` is an open text[] validated in app code
--                      (src/lib/api/scopes.ts) — same pattern as
--                      decision_records.applied_persona, so new scopes
--                      land without a migration. Default expiry of 180
--                      days is set in app code, NOT here, so staff can
--                      mint no-expire tokens per case.
--
--   api_token_usage    one row per (token, UTC date), holding the per-
--                      scope counters that back the daily rate limits.
--                      Counters bumped via INSERT … ON CONFLICT DO
--                      UPDATE in src/lib/api/rate-limit.ts. Pruning of
--                      old buckets is NOT yet implemented — the table
--                      will grow at one row per (active_token, day).
--                      Add a dedicated cron route when volume warrants
--                      (target: keep last 90 days, drop the rest).
--
-- Active-token check (used by every API request):
--
--   revoked_at IS NULL
--   AND (expires_at IS NULL OR expires_at > now())
--
-- The unique index on hashed_secret backs constant-time lookup. The
-- (user_id, created_at desc) index backs the /settings/tokens list.

CREATE TABLE "api_tokens" (
  "id"             uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  "user_id"        uuid NOT NULL REFERENCES "users"("id") ON DELETE CASCADE,
  "name"           text NOT NULL,
  "display_prefix" text NOT NULL,
  "hashed_secret"  text NOT NULL,
  "scopes"         text[] NOT NULL DEFAULT '{}',
  "last_used_at"   timestamptz,
  "expires_at"     timestamptz,
  "revoked_at"     timestamptz,
  "created_at"     timestamptz NOT NULL DEFAULT now()
);
--> statement-breakpoint

CREATE UNIQUE INDEX "idx_api_tokens_hashed_secret"
  ON "api_tokens" ("hashed_secret");
--> statement-breakpoint

CREATE INDEX "idx_api_tokens_user"
  ON "api_tokens" ("user_id", "created_at" DESC);
--> statement-breakpoint

CREATE TABLE "api_token_usage" (
  "token_id"           uuid NOT NULL REFERENCES "api_tokens"("id") ON DELETE CASCADE,
  "bucket_date"        date NOT NULL,
  "submissions_count"  integer NOT NULL DEFAULT 0,
  "comments_count"     integer NOT NULL DEFAULT 0,
  "votes_count"        integer NOT NULL DEFAULT 0,
  "saves_count"        integer NOT NULL DEFAULT 0,
  "reads_count"        integer NOT NULL DEFAULT 0,
  PRIMARY KEY ("token_id", "bucket_date")
);
