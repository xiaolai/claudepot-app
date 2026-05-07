-- 0028_cost_caps_and_invoices — bot cost-report stage 3.
--
-- Two pieces:
--
-- 1. Per-bot monthly USD cap. A nullable column on `users` (only
--    meaningful for is_agent=true accounts; null = no cap, the
--    common case). When a `kind='cost'` report pushes month-to-date
--    spend past the cap, persistBotReport emits a single
--    `kind='alert'` report tagged with key='cap_breach:YYYY-MM'.
--    The partial unique index below dedupes those alerts so only
--    the first cross of a given month-cap pair fires.
--
-- 2. Provider invoice ledger. Staff manually uploads one row per
--    (provider, month) with the invoiced USD figure. The /admin/
--    console/cost-reconcile page joins this against the
--    bot_costs_daily rollup to surface the diff between
--    self-reported and invoiced spend. Notes column lets staff
--    record context (rate cards, credits applied, etc.).
--
-- No backfill: existing bots have no cap until staff sets one;
-- existing months have no invoices until staff uploads them.

-- 1. monthly_usd_cap on users
ALTER TABLE "users"
  ADD COLUMN IF NOT EXISTS "monthly_usd_cap" numeric(10, 2);
--> statement-breakpoint

-- 2. Alert dedup. Same shape as idx_bot_reports_open_proposal_key,
--    but scoped to kind='alert'. ON CONFLICT DO NOTHING on the
--    server-side alert insert collapses repeat triggers within the
--    same month onto a single row.
CREATE UNIQUE INDEX IF NOT EXISTS "idx_bot_reports_alert_key"
  ON "bot_reports" ("bot_id", (("payload"->>'key')))
  WHERE "kind" = 'alert' AND "payload"->>'key' IS NOT NULL;
--> statement-breakpoint

-- 3. provider_invoices
CREATE TABLE IF NOT EXISTS "provider_invoices" (
  "id"           uuid PRIMARY KEY DEFAULT gen_random_uuid(),
  "provider"     text NOT NULL,
  "month"        text NOT NULL,
  "invoiced_usd" numeric(10, 2) NOT NULL,
  "uploaded_by"  uuid REFERENCES "users"("id") ON DELETE SET NULL,
  "uploaded_at"  timestamptz NOT NULL DEFAULT now(),
  "notes"        text,
  UNIQUE ("provider", "month")
);--> statement-breakpoint

CREATE INDEX IF NOT EXISTS "idx_provider_invoices_month"
  ON "provider_invoices" ("month" DESC);
