-- 0009_persona_bots — make ada/historian/scout real bot users so personas
-- aren't just labels on decision_records.
--
-- Background: rubric.yml v0.2.3 defines four editorial personas (ada,
-- shannon, historian, scout). Pre-this-migration, only `shannon` was a
-- real bot user (is_agent=true, role=system); `ada` was a seeded human-
-- rotation byline from foxed/to_shannon.py, and `historian`/`scout` did
-- not exist at all. Decision_records.applied_persona could carry strings
-- with no matching user record.
--
-- After this migration, every persona resolves to a real users row, so
-- /u/<persona> works as a profile page and submissions can be bylined by
-- the persona that scored them.

-- 1. Promote ada from human-rotation user to editorial-agent.
--    Fixture submissions previously bylined by ada become agent-bylined,
--    which is honest for a synthetic dev corpus and doesn't change any
--    production behavior.
UPDATE "users"
   SET role = 'system'::user_role,
       is_agent = true,
       updated_at = NOW()
 WHERE username = 'ada';--> statement-breakpoint

-- 2. Create historian and scout as is_agent=true bots. Use ON CONFLICT
--    DO NOTHING so re-runs are safe (the username unique constraint
--    catches the second invocation cleanly).
INSERT INTO "users"
  (username, name, email, role, is_agent, karma, created_at, updated_at)
VALUES
  ('historian', 'historian', 'historian@sha.nnon.ai.bot', 'system'::user_role, true, 0, NOW(), NOW()),
  ('scout',     'scout',     'scout@sha.nnon.ai.bot',     'system'::user_role, true, 0, NOW(), NOW())
ON CONFLICT (username) DO NOTHING;
