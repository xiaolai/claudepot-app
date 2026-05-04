-- 0014_digest_sends — idempotency guard for the weekly digest cron.
--
-- One row per (user_id, week_key). The cron does
--   INSERT INTO digest_sends (...) ON CONFLICT DO NOTHING RETURNING user_id
-- and only emails recipients whose insert produced a row. A rerun in
-- the same week (manual replay, Vercel cron retry on transient failure,
-- partial-failure restart) cannot deliver a second copy — the conflict
-- on the composite PK absorbs it.
--
-- week_key is text in ISO-8601 form ("2026-W18"). Text rather than
-- date so retries crossing the Sun→Mon UTC midnight still collapse
-- onto the same row — the runtime computes the key from the cron
-- fire time, not from "today".

CREATE TABLE "digest_sends" (
  "user_id"  uuid NOT NULL REFERENCES "users"("id") ON DELETE CASCADE,
  "week_key" text NOT NULL,
  "sent_at"  timestamptz NOT NULL DEFAULT now(),
  PRIMARY KEY ("user_id", "week_key")
);
