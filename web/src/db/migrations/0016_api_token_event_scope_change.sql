-- 0016_api_token_event_scope_change — extend the PAT audit enum.
--
-- 0013 created api_token_events.event as {mint, revoke}. The slice-2
-- scope additions (submission:delete, comment:delete on 2026-05-06)
-- required an in-place UPDATE of api_tokens.scopes for the office bot
-- fleet — see web/scripts/refresh-bot-scopes.ts. That class of
-- mutation needs its own audit variant so the trail does not collapse
-- onto "mint" or vanish entirely.
--
-- IF NOT EXISTS makes this safe to re-run on an already-migrated DB.
-- ALTER TYPE … ADD VALUE cannot run inside a transaction block, so
-- the migration script (apply-migration.ts) must execute this as its
-- own auto-commit statement; the Neon HTTP driver does that already.

ALTER TYPE "api_token_event" ADD VALUE IF NOT EXISTS 'scope_change';
