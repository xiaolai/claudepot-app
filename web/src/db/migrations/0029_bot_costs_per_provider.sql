-- 0029_bot_costs_per_provider — extend bot_costs_daily PK with provider.
--
-- Without provider in the PK, Anthropic + OpenAI charges on the same
-- day for the same bot collapse into one row, and per-provider
-- invoice reconciliation can't match self-reports to invoices.
--
-- Migration shape:
--   1. Drop the (bot_id, day) PK.
--   2. Add `provider text NOT NULL` with a backfill default of
--      'anthropic' — covers any rows that existed in non-prod
--      environments. In prod the table is empty (verified before
--      authoring this migration) so the default touches no rows.
--   3. Drop the column default; new inserts MUST specify provider
--      explicitly. The cron and the schema declaration enforce this.
--   4. Add the new (bot_id, day, provider) PK.
--   5. Same shape for the new "rolled up" tracking column —
--      rolled_up_at stays unchanged.
--
-- Bot-side: cost reports already include payload.provider per
-- lib/bots/schemas.ts:costPayloadSchema. The cron extracts it and
-- groups by (bot, day, provider).

ALTER TABLE "bot_costs_daily" DROP CONSTRAINT "bot_costs_daily_pkey";
--> statement-breakpoint

ALTER TABLE "bot_costs_daily"
  ADD COLUMN "provider" text NOT NULL DEFAULT 'anthropic';
--> statement-breakpoint

ALTER TABLE "bot_costs_daily" ALTER COLUMN "provider" DROP DEFAULT;
--> statement-breakpoint

ALTER TABLE "bot_costs_daily"
  ADD PRIMARY KEY ("bot_id", "day", "provider");
