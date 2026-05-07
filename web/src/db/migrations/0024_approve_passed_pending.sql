-- 0024_approve_passed_pending — retroactively approve pending
-- submissions and comments that Ada already passed.
--
-- Companion to the karma-gate disable in
-- src/lib/submissions/state.ts and src/lib/comments/state.ts
-- (KARMA_GATE_ENABLED=false in both). The gates held content in
-- 'pending' even when the policy moderator had passed them, because
-- the karma signal they were checking didn't exist yet on a
-- launch-mode site. The runtime change is prospective; this
-- migration applies the same logic retroactively so user permalinks
-- aren't 404 for content the moderator already cleared.
--
-- Scope: only rows that satisfy ALL of the following:
--   - state = 'pending' (not already approved/rejected)
--   - deleted_at IS NULL (don't resurrect tombstones)
--   - at least one policy_decisions row exists for this submission
--     with verdict='pass' (Ada explicitly passed it; not a
--     synthetic-error or capped or disabled verdict that wrote no
--     policy_decisions row)
--
-- Pending submissions WITHOUT a passing policy_decisions row are
-- left alone — those were either pre-Ada (no decision exists),
-- moderator-errored (synthetic verdict, no row written per
-- lib/moderation/persist.ts), or held for staff. Each of those
-- needs eyes, not a blanket flip.
--
-- published_at is set to the moderation decision time, not NOW(),
-- so the feed ordering reflects when the post was actually cleared
-- rather than when this migration ran. Falls back to NOW() if for
-- any reason the policy_decisions row is missing decided_at, which
-- shouldn't happen (column is NOT NULL DEFAULT now()).
--
-- Idempotent: subsequent runs match no rows because the WHERE
-- clause excludes already-approved submissions.

UPDATE "submissions" AS s
   SET "state" = 'approved',
       "published_at" = COALESCE(
         s.published_at,
         (
           SELECT MIN(pd.decided_at)
             FROM "policy_decisions" pd
            WHERE pd.target_type = 'submission'
              AND pd.target_id = s.id
              AND pd.verdict = 'pass'
         ),
         NOW()
       )
 WHERE s.state = 'pending'
   AND s.deleted_at IS NULL
   AND EXISTS (
     SELECT 1
       FROM "policy_decisions" pd
      WHERE pd.target_type = 'submission'
        AND pd.target_id = s.id
        AND pd.verdict = 'pass'
   );
--> statement-breakpoint

-- Same idea for comments. Comments table has no published_at column
-- (the row is itself the published artifact); only the state flip
-- is needed.
UPDATE "comments" AS c
   SET "state" = 'approved'
 WHERE c.state = 'pending'
   AND c.deleted_at IS NULL
   AND EXISTS (
     SELECT 1
       FROM "policy_decisions" pd
      WHERE pd.target_type = 'comment'
        AND pd.target_id = c.id
        AND pd.verdict = 'pass'
   );
