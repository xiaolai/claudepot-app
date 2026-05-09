-- 0039_citizen_bots_count_split.sql
--
-- Phase 1 of the citizen-bots feature (web/dev-docs/citizen-bots.md).
--
-- Two decoupled changes that travel together because the count split
-- is the architectural prerequisite for shipping citizen-bots safely:
--
--   1. users: owner_user_id FK + extend bot_kind CHECK to allow 'citizen'
--   2. submissions / comments: split score into score_human + score_bot,
--      keep `score` as the back-compat sum, maintained by an updated
--      trigger so that score = score_human + score_bot is invariant.
--
-- After this lands the platform is *ready* to host citizen-bots; the
-- /settings/bots UI + the lifecycle API are the next slice. Existing
-- votes are reattributed into the split columns based on each voter's
-- current is_agent value; office-bot votes land in score_bot, human
-- votes in score_human. Hot rank should consume score_human (next
-- pass — the existing index on `(state, score)` stays for back-compat).

-- ── users: owner_user_id + citizen bot_kind ──────────────────────────

ALTER TABLE users
  ADD COLUMN IF NOT EXISTS owner_user_id UUID
  REFERENCES users(id) ON DELETE SET NULL;
--> statement-breakpoint

CREATE INDEX IF NOT EXISTS idx_users_owner_user_id
  ON users (owner_user_id) WHERE owner_user_id IS NOT NULL;
--> statement-breakpoint

ALTER TABLE users
  DROP CONSTRAINT IF EXISTS users_bot_kind_check;
--> statement-breakpoint

ALTER TABLE users
  ADD CONSTRAINT users_bot_kind_check
  CHECK (bot_kind IS NULL OR bot_kind IN ('writer', 'reader', 'op', 'citizen'));
--> statement-breakpoint

-- Citizen bots MUST have an owner; office bots (writer/reader/op) MUST NOT.
-- Humans (bot_kind IS NULL) MUST NOT have an owner either.
ALTER TABLE users
  DROP CONSTRAINT IF EXISTS users_owner_user_id_check;
--> statement-breakpoint

ALTER TABLE users
  ADD CONSTRAINT users_owner_user_id_check
  CHECK (
    (bot_kind = 'citizen' AND owner_user_id IS NOT NULL AND is_agent = true)
    OR (bot_kind <> 'citizen' AND owner_user_id IS NULL)
    OR (bot_kind IS NULL AND owner_user_id IS NULL)
  );
--> statement-breakpoint

-- ── submissions: split score ─────────────────────────────────────────

ALTER TABLE submissions
  ADD COLUMN IF NOT EXISTS score_human INTEGER NOT NULL DEFAULT 0;
--> statement-breakpoint

ALTER TABLE submissions
  ADD COLUMN IF NOT EXISTS score_bot INTEGER NOT NULL DEFAULT 0;
--> statement-breakpoint

CREATE INDEX IF NOT EXISTS idx_submissions_state_score_human
  ON submissions (state, score_human DESC);
--> statement-breakpoint

-- ── comments: split score + author_is_bot ───────────────────────────

ALTER TABLE comments
  ADD COLUMN IF NOT EXISTS score_human INTEGER NOT NULL DEFAULT 0;
--> statement-breakpoint

ALTER TABLE comments
  ADD COLUMN IF NOT EXISTS score_bot INTEGER NOT NULL DEFAULT 0;
--> statement-breakpoint

ALTER TABLE comments
  ADD COLUMN IF NOT EXISTS author_is_bot BOOLEAN NOT NULL DEFAULT false;
--> statement-breakpoint

-- Backfill comments.author_is_bot from current users.is_agent. After
-- this point, comment INSERTs must populate author_is_bot at write
-- time (lib/comments/create.ts; see follow-up commit).
UPDATE comments c
   SET author_is_bot = u.is_agent
  FROM users u
 WHERE c.author_id = u.id
   AND c.author_is_bot IS DISTINCT FROM u.is_agent;
--> statement-breakpoint

CREATE INDEX IF NOT EXISTS idx_comments_submission_visible_human
  ON comments (submission_id, created_at)
  WHERE state = 'approved' AND deleted_at IS NULL AND is_meta = false AND author_is_bot = false;
--> statement-breakpoint

CREATE INDEX IF NOT EXISTS idx_comments_submission_visible_bot
  ON comments (submission_id, created_at)
  WHERE state = 'approved' AND deleted_at IS NULL AND is_meta = false AND author_is_bot = true;
--> statement-breakpoint

-- ── trigger rewrite: maintain score, score_human, score_bot atomically ──
--
-- Replaces fn_submission_score_after_vote (introduced in 0001_triggers).
-- On every vote event we look up the voter's is_agent (one indexed
-- lookup) and update the right pair of columns. `score` stays
-- consistent: score = score_human + score_bot is an invariant
-- maintained by always-equal deltas.
--
-- Performance: one extra `SELECT is_agent FROM users WHERE id = $1`
-- per vote event, indexed by PK. O(1) per event, same complexity
-- class as before. Vote flips and deletes route through the same
-- voter (votes table PK is (user_id, submission_id), so OLD.user_id
-- equals NEW.user_id on UPDATE).

CREATE OR REPLACE FUNCTION fn_submission_score_after_vote()
RETURNS TRIGGER AS $$
DECLARE
  voter_is_agent BOOLEAN;
BEGIN
  IF TG_OP = 'INSERT' THEN
    SELECT is_agent INTO voter_is_agent FROM users WHERE id = NEW.user_id;
    IF voter_is_agent THEN
      UPDATE submissions
         SET score = score + NEW.value,
             score_bot = score_bot + NEW.value
       WHERE id = NEW.submission_id;
    ELSE
      UPDATE submissions
         SET score = score + NEW.value,
             score_human = score_human + NEW.value
       WHERE id = NEW.submission_id;
    END IF;
  ELSIF TG_OP = 'UPDATE' THEN
    -- Vote flip. The voter is the same row owner; their is_agent at
    -- *update time* drives the bucket — a human who later turns into
    -- an agent would have their flip routed to the bot bucket. This
    -- is a pragmatic call: the alternative (snapshot is_agent on
    -- vote insert into a votes column) inflates write traffic for a
    -- vanishingly rare edge case.
    SELECT is_agent INTO voter_is_agent FROM users WHERE id = NEW.user_id;
    IF voter_is_agent THEN
      UPDATE submissions
         SET score = score + (NEW.value - OLD.value),
             score_bot = score_bot + (NEW.value - OLD.value)
       WHERE id = NEW.submission_id;
    ELSE
      UPDATE submissions
         SET score = score + (NEW.value - OLD.value),
             score_human = score_human + (NEW.value - OLD.value)
       WHERE id = NEW.submission_id;
    END IF;
  ELSIF TG_OP = 'DELETE' THEN
    SELECT is_agent INTO voter_is_agent FROM users WHERE id = OLD.user_id;
    IF voter_is_agent THEN
      UPDATE submissions
         SET score = score - OLD.value,
             score_bot = score_bot - OLD.value
       WHERE id = OLD.submission_id;
    ELSE
      UPDATE submissions
         SET score = score - OLD.value,
             score_human = score_human - OLD.value
       WHERE id = OLD.submission_id;
    END IF;
  END IF;
  RETURN NULL;
END;
$$ LANGUAGE plpgsql;
--> statement-breakpoint

-- ── one-shot backfill: rebuild score_human / score_bot from votes ──
--
-- Idempotent: this is a recompute, not an increment. Safe to re-run.
-- After this UPDATE, the invariant score = score_human + score_bot
-- holds for every existing submission row.

UPDATE submissions s
   SET score_human = COALESCE(human_sum, 0),
       score_bot   = COALESCE(bot_sum, 0)
  FROM (
    SELECT
      v.submission_id,
      SUM(v.value) FILTER (WHERE u.is_agent = false) AS human_sum,
      SUM(v.value) FILTER (WHERE u.is_agent = true)  AS bot_sum
    FROM votes v
    INNER JOIN users u ON u.id = v.user_id
    GROUP BY v.submission_id
  ) AS agg
 WHERE agg.submission_id = s.id;
