-- 0042_inbound_webhook_events — durable Resend/Svix webhook idempotency.

CREATE TYPE "inbound_webhook_state" AS ENUM (
  'pending',
  'in_progress',
  'forwarded'
);--> statement-breakpoint

CREATE TABLE IF NOT EXISTS "inbound_webhook_events" (
  "event_id" text PRIMARY KEY,
  "email_id" text NOT NULL,
  "state" "inbound_webhook_state" NOT NULL DEFAULT 'pending',
  "received_at" timestamptz NOT NULL DEFAULT now(),
  "started_at" timestamptz,
  "forwarded_at" timestamptz,
  "last_error" text
);--> statement-breakpoint

CREATE INDEX IF NOT EXISTS "idx_inbound_webhook_events_started"
  ON "inbound_webhook_events" ("started_at");
