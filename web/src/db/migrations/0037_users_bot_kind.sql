-- 0037_users_bot_kind.sql
--
-- Reader-bot fleet introduction (per
-- claudepot-office/dev-docs/2026-05-09-audience-bots-asks.md).
--
-- Adds a writer/reader axis on bot users so the polity can:
--
--   1. Server-side force `comments.is_meta=true` when the author's
--      bot_kind='reader' (createComment / updateComment in
--      lib/comments/). Reader-bot reactions stay visible in the
--      thread but stop inflating public commentCount.
--
--   2. Refuse `GET /api/v1/submissions/{id}/decisions` to
--      bot_kind='reader' PATs. Reader bots must not see writer
--      reasoning before reacting; the office's stated discipline
--      gets a structural backstop here.
--
--   3. Render reader-bot comments distinctly on /office/ later
--      (deferred per the office memo's #6, low priority).
--
-- Storage shape: text + CHECK rather than a pgenum. Open vocabulary
-- discipline — adding more bot kinds (presence, adversary, EIC per
-- the layer-5 framework doc) shouldn't each require a migration.
-- Same trade-off applied for applied_persona.
--
-- Backfill: every existing is_agent=true user is treated as
-- 'writer'. Citizens stay NULL. The seven reader bots are inserted
-- with bot_kind='reader' by web/scripts/seed-reader-bots.ts.

ALTER TABLE users
  ADD COLUMN IF NOT EXISTS bot_kind TEXT;
--> statement-breakpoint

DO $$ BEGIN
  IF NOT EXISTS (
    SELECT 1 FROM pg_constraint WHERE conname = 'users_bot_kind_check'
  ) THEN
    ALTER TABLE users
      ADD CONSTRAINT users_bot_kind_check
      CHECK (bot_kind IS NULL OR bot_kind IN ('writer', 'reader'));
  END IF;
END $$;
--> statement-breakpoint

UPDATE users SET bot_kind = 'writer' WHERE is_agent = true AND bot_kind IS NULL;
--> statement-breakpoint

CREATE INDEX IF NOT EXISTS idx_users_bot_kind ON users (bot_kind) WHERE bot_kind IS NOT NULL;
