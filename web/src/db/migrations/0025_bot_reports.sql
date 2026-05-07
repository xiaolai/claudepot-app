-- 0025_bot_reports — bot self-reporting substrate.
--
-- 15 office bots (Ada moderates, Warren writes, the rest land within
-- days) need a single canonical place to post:
--   * heartbeats   — "I'm alive, here's my version"
--   * work_summary — rolled-up batch of work units done in a window
--   * cost         — token spend per provider + model
--   * error        — non-fatal but operator-worthy
--   * proposal     — bot wants a human ack on a change (vocab, block,
--                    tag merge, etc.) — surfaces in the /admin Today
--                    inbox notice strip alongside Ada's pending tags
--   * decision_summary — moderation-class bots self-report verdict +
--                    confidence distribution + drift z-score
--
-- Two tables, on purpose:
--
-- bot_heartbeats: one row per bot, UPSERTed. Heartbeats are
-- high-frequency and only the latest one matters. Storing every
-- ping would be wasteful at 15 bots × per-minute cadence.
--
-- bot_reports: append-only event log. One row per work_summary /
-- cost / error / proposal / decision_summary report. cost_usd is
-- denormalized out of the payload so the dashboard's 7d/30d
-- roll-ups SUM in one query without jsonb extraction.
--
-- Auth model: each bot has its own api_tokens row with the new
-- bots:report scope. The endpoint reads auth.user_id and uses it
-- as the bot_id — there's no bot_id field in the request body, so
-- a leaked token can only post for the bot it belongs to.
--
-- Why a partial unique index on open proposals: a bot might re-post
-- the same proposal across runs (idempotency). The
-- (bot_id, kind, payload->>'target_id') tuple identifies a proposal
-- uniquely while it's still open; once staff acts (status flips to
-- 'accepted' or 'rejected'), a new proposal can land.

CREATE TABLE IF NOT EXISTS "bot_heartbeats" (
  "bot_id"        uuid PRIMARY KEY REFERENCES "users"("id") ON DELETE CASCADE,
  "version"       text,
  "env"           text,
  "last_seen_at"  timestamptz NOT NULL DEFAULT now(),
  "meta"          jsonb
);
--> statement-breakpoint

CREATE TABLE IF NOT EXISTS "bot_reports" (
  "id"            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  "bot_id"        uuid NOT NULL REFERENCES "users"("id") ON DELETE CASCADE,
  -- Open enum kept as text. App layer (lib/bots/schemas.ts) is the
  -- authoritative whitelist; the API rejects unknown kinds at the
  -- boundary so this column never carries garbage.
  "kind"          text NOT NULL,
  "payload"       jsonb NOT NULL DEFAULT '{}'::jsonb,
  -- Denormalized out of payload for fast SUM() roll-ups across 15
  -- bots × thousands of rows. NULL when the report kind isn't
  -- cost-bearing (heartbeat / proposal / error usually).
  "cost_usd"      numeric(10, 6),
  -- Proposal-only. NULL for non-proposal kinds. Flips to 'accepted'
  -- or 'rejected' when staff acts via the inbox notice strip; stays
  -- 'open' until then.
  "status"        text,
  "resolved_by"   uuid REFERENCES "users"("id") ON DELETE SET NULL,
  "resolved_at"   timestamptz,
  "reported_at"   timestamptz NOT NULL DEFAULT now()
);
--> statement-breakpoint

-- Per-bot timeline (drill page) — DESC so the dashboard's "latest"
-- queries hit the index head.
CREATE INDEX IF NOT EXISTS "idx_bot_reports_bot_reported"
  ON "bot_reports" ("bot_id", "reported_at" DESC);
--> statement-breakpoint

-- Cross-bot cost roll-ups (Health page, console index card stats).
-- Filtered to cost-bearing rows so the index stays small.
CREATE INDEX IF NOT EXISTS "idx_bot_reports_cost_reported"
  ON "bot_reports" ("reported_at" DESC)
  WHERE "cost_usd" IS NOT NULL;
--> statement-breakpoint

-- Inbox notice strip — open proposals across all bots, oldest
-- first (operator clears them in age order).
CREATE INDEX IF NOT EXISTS "idx_bot_reports_open_proposals"
  ON "bot_reports" ("reported_at")
  WHERE "kind" = 'proposal' AND "status" = 'open';
--> statement-breakpoint

-- Idempotency: a bot can re-post the same proposal across retries
-- without spawning duplicates while it's still open. payload->>'key'
-- is the bot's own opaque dedup key — bots that don't supply one
-- get free duplicate-spawning, which is fine for one-shot proposals.
CREATE UNIQUE INDEX IF NOT EXISTS "idx_bot_reports_open_proposal_key"
  ON "bot_reports" ("bot_id", ("payload"->>'key'))
  WHERE "kind" = 'proposal' AND "status" = 'open' AND "payload"->>'key' IS NOT NULL;
--> statement-breakpoint

-- Rate-limit bucket for the bots:report scope. Same per-token,
-- per-day shape as the existing buckets in api_token_usage. Adding
-- a column on a small table; default 0 keeps existing rows valid.
ALTER TABLE "api_token_usage"
  ADD COLUMN IF NOT EXISTS "bots_count" integer NOT NULL DEFAULT 0;
