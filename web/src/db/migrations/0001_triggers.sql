-- Hand-written migration: incremental triggers for `submissions.score`
-- and `users.karma`. Drizzle-kit doesn't emit triggers; these live here
-- alongside the schema migrations and follow the same up/down lifecycle.
--
-- Both triggers are O(1) per event. Do NOT change them to recompute via
-- SUM(...) — that becomes O(N) per vote and dominates DB CPU at any
-- real volume (see design/architecture.md §4 for the spec).

-- ── submissions.score = sum(votes.value) for that submission ──────────

CREATE OR REPLACE FUNCTION fn_submission_score_after_vote()
RETURNS TRIGGER AS $$
BEGIN
  IF TG_OP = 'INSERT' THEN
    UPDATE submissions
       SET score = score + NEW.value
     WHERE id = NEW.submission_id;
  ELSIF TG_OP = 'UPDATE' THEN
    -- Vote flip (e.g. up → down, or null → up).
    UPDATE submissions
       SET score = score + (NEW.value - OLD.value)
     WHERE id = NEW.submission_id;
  ELSIF TG_OP = 'DELETE' THEN
    UPDATE submissions
       SET score = score - OLD.value
     WHERE id = OLD.submission_id;
  END IF;
  RETURN NULL;
END;
$$ LANGUAGE plpgsql;
--> statement-breakpoint

CREATE TRIGGER trg_submission_score_after_vote
AFTER INSERT OR UPDATE OR DELETE ON votes
FOR EACH ROW EXECUTE FUNCTION fn_submission_score_after_vote();
--> statement-breakpoint

-- ── users.karma = sum(score) over the user's submissions + comments ──

CREATE OR REPLACE FUNCTION fn_user_karma_after_submission_score()
RETURNS TRIGGER AS $$
BEGIN
  IF NEW.score IS DISTINCT FROM OLD.score THEN
    UPDATE users
       SET karma = karma + (NEW.score - OLD.score)
     WHERE id = NEW.author_id;
  END IF;
  RETURN NULL;
END;
$$ LANGUAGE plpgsql;
--> statement-breakpoint

CREATE TRIGGER trg_user_karma_after_submission_score
AFTER UPDATE OF score ON submissions
FOR EACH ROW EXECUTE FUNCTION fn_user_karma_after_submission_score();
--> statement-breakpoint

CREATE OR REPLACE FUNCTION fn_user_karma_after_comment_score()
RETURNS TRIGGER AS $$
BEGIN
  IF NEW.score IS DISTINCT FROM OLD.score THEN
    UPDATE users
       SET karma = karma + (NEW.score - OLD.score)
     WHERE id = NEW.author_id;
  END IF;
  RETURN NULL;
END;
$$ LANGUAGE plpgsql;
--> statement-breakpoint

CREATE TRIGGER trg_user_karma_after_comment_score
AFTER UPDATE OF score ON comments
FOR EACH ROW EXECUTE FUNCTION fn_user_karma_after_comment_score();
