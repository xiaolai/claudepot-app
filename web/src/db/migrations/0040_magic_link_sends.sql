-- 0040_magic_link_sends — DB-backed throttle for magic-link email sends.
--
-- The /login magic-link form (and the raw /api/auth/signin/resend
-- endpoint behind it) sends paid Resend email to an arbitrary,
-- attacker-chosen address. This table backs the fixed-window throttle
-- enforced by src/lib/magic-link-rate-limit.ts from the Auth.js signIn
-- callback: one row per (key, UTC hour bucket), where key is
-- "email:<normalized address>" or "ip:<client ip>".
--
-- Same INSERT … ON CONFLICT DO UPDATE … RETURNING pattern as
-- api_token_usage (src/lib/api/rate-limit.ts). Rows are pruned
-- opportunistically on each send once older than 24 hours, so the
-- table stays a handful of rows.

CREATE TABLE "magic_link_sends" (
  "key"         text NOT NULL,
  "bucket_hour" timestamptz NOT NULL,
  "count"       integer NOT NULL DEFAULT 1,
  PRIMARY KEY ("key", "bucket_hour")
);
