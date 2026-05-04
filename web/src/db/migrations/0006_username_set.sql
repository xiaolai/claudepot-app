-- First-sign-in OAuth users get a placeholder username and must pick a
-- real one through the /onboarding/username flow. `username_set` is the
-- gate: false → redirected to onboarding; true → cleared.
-- Default true so seeded fixture users and any rows that already existed
-- before this migration are exempt from the new flow.

ALTER TABLE "users" ADD COLUMN IF NOT EXISTS "username_set" boolean NOT NULL DEFAULT true;
