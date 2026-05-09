-- 0038_bot_kind_op.sql
--
-- Extend the users.bot_kind CHECK constraint to allow 'op' alongside
-- 'writer' and 'reader'.
--
-- Background: 0037 introduced the writer/reader axis for the seven
-- audience bots. The office now has a third bot class — operators —
-- whose only API surface is bot self-reporting (heartbeat, cost,
-- error). The canary is `otto@daemon` (bot-team-frameworks memo,
-- ~/claudepot-office/bots/otto@daemon/). Op bots:
--
--   - hold a single scope: bots:report
--   - never author submissions or comments
--   - never read other users' content (no read:all)
--   - act as infra canaries / health probes / cost loggers
--
-- Same trade-off as 0037 (text + CHECK rather than pgenum) — adding a
-- fourth kind later (e.g. 'eic', 'presence' per the layer-5 framework
-- memo) is one DDL line, not a value-juggling enum migration.
--
-- No backfill: existing rows are 'writer', 'reader', or NULL. Op bots
-- (including otto) are inserted by web/scripts/seed-op-bots.ts with
-- bot_kind='op'.

ALTER TABLE users
  DROP CONSTRAINT IF EXISTS users_bot_kind_check;
--> statement-breakpoint

ALTER TABLE users
  ADD CONSTRAINT users_bot_kind_check
  CHECK (bot_kind IS NULL OR bot_kind IN ('writer', 'reader', 'op'));
