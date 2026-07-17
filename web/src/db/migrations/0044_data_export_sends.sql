-- 0044_data_export_sends — DB-backed throttle for data-export emails.
--
-- The settings-page "Export my data" action (requestDataExport in
-- src/lib/actions/settings.ts) dumps the user's rows and sends a paid
-- (Resend) email per call. Unthrottled, a held-down submit button
-- burns Resend quota and hammers the DB with full-table-per-user
-- scans. This table backs a per-user fixed-window cap of 2 sends per
-- UTC day, enforced by src/lib/data-export-rate-limit.ts.
--
-- Same INSERT … ON CONFLICT DO UPDATE … RETURNING pattern as
-- magic_link_sends (migration 0040). Rows are pruned opportunistically
-- on each send once older than 48 hours, so the table stays tiny.
--
-- Apply via `pnpm exec tsx --env-file=.env.local
-- scripts/apply-migration.ts src/db/migrations/0044_data_export_sends.sql`
-- per .claude/rules/db-migrations.md. NEVER drizzle-kit push.

CREATE TABLE "data_export_sends" (
  "user_id"    uuid NOT NULL REFERENCES "users"("id") ON DELETE CASCADE,
  "bucket_day" timestamptz NOT NULL,
  "count"      integer NOT NULL DEFAULT 1,
  PRIMARY KEY ("user_id", "bucket_day")
);
