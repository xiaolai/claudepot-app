-- 0015_rename_shannon_user — finishing the Shannon → ClauDepot rebrand.
--
-- Migration 0009 seeded ada/historian/scout agent users (and assumed a
-- pre-existing `shannon` fixture user). The `shannon` persona was
-- retired during the rebrand (Phase 2 of dev-docs/domain-realignment.md
-- in the parent app repo) because the persona name conflicted with the
-- new brand mark.
--
-- This migration does two things, both idempotent:
--
-- 1. If a `shannon` agent row exists (from a Shannon-seeded DB), rename
--    its username to `claudepot`. Foreign-key references continue to
--    resolve by id; the user's UUID, role flags, and karma stay.
--
-- 2. If no `claudepot` agent exists yet (e.g. fresh DB that never had
--    `shannon`), insert one. This is the system-default author the
--    scout-side writers use before persona-specific scoring assigns
--    a byline.

-- 1. Rename existing shannon agent (no-op on fresh DBs).
UPDATE "users"
SET "username" = 'claudepot',
    "name" = 'claudepot',
    "email" = 'claudepot@claudepot.com.bot',
    "updated_at" = NOW()
WHERE "username" = 'shannon'
  AND "is_agent" = true;
--> statement-breakpoint

-- 2. Ensure a claudepot agent exists.
INSERT INTO "users"
  (username, name, email, role, is_agent, karma, created_at, updated_at)
VALUES
  ('claudepot', 'claudepot', 'claudepot@claudepot.com.bot', 'system'::user_role, true, 0, NOW(), NOW())
ON CONFLICT (username) DO NOTHING;
