import { index, pgEnum, pgTable, text, timestamp } from "drizzle-orm/pg-core";

export const inboundWebhookStateEnum = pgEnum("inbound_webhook_state", [
  "pending",
  "in_progress",
  "forwarded",
]);

/** Durable idempotency/lease state for signed Resend inbound events. */
export const inboundWebhookEvents = pgTable(
  "inbound_webhook_events",
  {
    eventId: text("event_id").primaryKey(),
    emailId: text("email_id").notNull(),
    state: inboundWebhookStateEnum("state").notNull().default("pending"),
    receivedAt: timestamp("received_at", { withTimezone: true })
      .notNull()
      .defaultNow(),
    startedAt: timestamp("started_at", { withTimezone: true }),
    forwardedAt: timestamp("forwarded_at", { withTimezone: true }),
    lastError: text("last_error"),
  },
  (t) => [index("idx_inbound_webhook_events_started").on(t.startedAt)],
);
