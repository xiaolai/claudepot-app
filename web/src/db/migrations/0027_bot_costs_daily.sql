-- 0027_bot_costs_daily — daily-cost rollup table populated by the
-- daily-rollup cron, read by /office/costs.
--
-- Why a rollup vs. continuing to query bot_reports directly:
--   * Independence from bot_reports retention. If we ever prune the
--     events table (current policy: keep, but the schema header
--     describes it as append-only-with-resolution-pruning, so future-
--     us may compress old rows), the cost history survives.
--   * /office/costs reads switch to the rollup for closed days; the
--     "today" row is computed live from bot_reports. The grand-total
--     and 90-day window thus survive any retention change.
--
-- Composite PK on (bot_id, day) makes the cron's nightly INSERT …
-- ON CONFLICT DO UPDATE idempotent: a retry — Vercel cron transient
-- failure, manual replay — collapses onto the same row.
--
-- The day column is `date` (no time, UTC-bucketed by the cron). This
-- is the same convention metrics_daily.day uses (text 'YYYY-MM-DD',
-- but date is the structurally cleaner type — picked here because
-- this is a new table and we don't have backwards compatibility to
-- preserve).

CREATE TABLE "bot_costs_daily" (
  "bot_id"       uuid NOT NULL REFERENCES "users"("id") ON DELETE CASCADE,
  "day"          date NOT NULL,
  "usd"          numeric(10, 6) NOT NULL DEFAULT 0,
  "reports"      integer NOT NULL DEFAULT 0,
  "rolled_up_at" timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY ("bot_id", "day")
);--> statement-breakpoint

CREATE INDEX "idx_bot_costs_daily_day" ON "bot_costs_daily" ("day" DESC);--> statement-breakpoint

-- Backfill from existing bot_reports so /office/costs has history
-- on day 1 — without this, the page would only show data from
-- whenever the next daily-rollup cron ticks. ON CONFLICT keeps the
-- backfill safe to re-run.
INSERT INTO "bot_costs_daily" ("bot_id", "day", "usd", "reports")
SELECT
  "bot_id",
  (date_trunc('day', "reported_at" AT TIME ZONE 'UTC'))::date AS day,
  COALESCE(SUM("cost_usd"), 0) AS usd,
  COUNT(*)::int AS reports
FROM "bot_reports"
WHERE "kind" = 'cost'
  AND "cost_usd" IS NOT NULL
GROUP BY 1, 2
ON CONFLICT ("bot_id", "day") DO UPDATE
SET "usd" = EXCLUDED."usd",
    "reports" = EXCLUDED."reports",
    "rolled_up_at" = now();
