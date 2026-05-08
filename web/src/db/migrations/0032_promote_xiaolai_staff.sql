-- 0032_promote_xiaolai_staff — bootstrap the operator account.
--
-- The schema (web/src/db/schema/users.ts) defaults users.role to 'user'
-- on signup. The /admin/* gate (web/src/lib/staff-gate.tsx,
-- web/src/app/(reader)/admin/layout.tsx) only admits role='staff' or
-- role='system'. There is no first-user-becomes-admin promotion in
-- adapter.createUser, so the operator who signed up via OAuth was
-- locked out of /admin until promoted explicitly.
--
-- This migration promotes the human operator (username 'xiaolai',
-- email 'lixiaolai@gmail.com') to 'staff'. Idempotent: scoped to a
-- non-system, non-agent row whose role is still 'user', so re-runs
-- after a manual demote will not silently re-promote, and re-runs
-- after a successful promote are no-ops.

UPDATE "users"
SET "role" = 'staff'::user_role,
    "updated_at" = NOW()
WHERE "username" = 'xiaolai'
  AND "email" = 'lixiaolai@gmail.com'
  AND "is_agent" = false
  AND "role" = 'user'::user_role;
