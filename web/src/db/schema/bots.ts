/**
 * Bot self-reporting substrate (migration 0025_bot_reports).
 *
 * Two tables:
 *
 *   - `bot_heartbeats` — UPSERTed liveness. One row per bot, only
 *     the latest matters.
 *   - `bot_reports` — append-only event log: work_summary, cost,
 *     error, proposal, decision_summary. `cost_usd` is denormalized
 *     out of `payload` so the Health page sums spend across 15 bots
 *     in one query instead of jsonb extraction per row. Proposals
 *     carry `status` ('open' | 'accepted' | 'rejected') and surface
 *     in the /admin Today inbox notice strip until staff acts.
 *
 * Auth: a bot's reports are tied to its `users.id` (bot_id), which
 * is derived from the api_tokens row carrying the request. There is
 * no bot_id field on the request — token leak isolates impact to
 * the one bot.
 *
 * The `kind` and `status` columns are open text enums (additive
 * without a migration). lib/bots/schemas.ts is the authoritative
 * whitelist and the API boundary rejects unknown values.
 */

import {
  date,
  index,
  integer,
  jsonb,
  numeric,
  pgTable,
  primaryKey,
  text,
  timestamp,
  uniqueIndex,
  uuid,
} from "drizzle-orm/pg-core";
import { sql } from "drizzle-orm";

import { users } from "./users";

export const botHeartbeats = pgTable("bot_heartbeats", {
  botId: uuid("bot_id")
    .primaryKey()
    .references(() => users.id, { onDelete: "cascade" }),
  version: text("version"),
  env: text("env"),
  lastSeenAt: timestamp("last_seen_at", { withTimezone: true })
    .notNull()
    .defaultNow(),
  meta: jsonb("meta"),
});

export const botReports = pgTable(
  "bot_reports",
  {
    id: uuid("id").primaryKey().defaultRandom(),
    botId: uuid("bot_id")
      .notNull()
      .references(() => users.id, { onDelete: "cascade" }),
    kind: text("kind").notNull(),
    payload: jsonb("payload").notNull().default(sql`'{}'::jsonb`),
    costUsd: numeric("cost_usd", { precision: 10, scale: 6 }),
    status: text("status"),
    resolvedBy: uuid("resolved_by").references(() => users.id, {
      onDelete: "set null",
    }),
    resolvedAt: timestamp("resolved_at", { withTimezone: true }),
    reportedAt: timestamp("reported_at", { withTimezone: true })
      .notNull()
      .defaultNow(),
  },
  (t) => [
    index("idx_bot_reports_bot_reported").on(t.botId, t.reportedAt.desc()),
    // Filtered indexes match the migration; the .where() clause is
    // declared on the schema to keep drizzle-kit's diff stable so
    // `push` doesn't try to drop and recreate them on every check.
    index("idx_bot_reports_cost_reported")
      .on(t.reportedAt.desc())
      .where(sql`${t.costUsd} IS NOT NULL`),
    index("idx_bot_reports_open_proposals")
      .on(t.reportedAt)
      .where(sql`${t.kind} = 'proposal' AND ${t.status} = 'open'`),
    uniqueIndex("idx_bot_reports_open_proposal_key")
      .on(t.botId, sql`(${t.payload}->>'key')`)
      .where(
        sql`${t.kind} = 'proposal' AND ${t.status} = 'open' AND ${t.payload}->>'key' IS NOT NULL`,
      ),
  ],
);

/**
 * Daily-cost rollup (migration 0027). One row per (bot_id, day);
 * populated by the daily-rollup cron each midnight UTC. /office/costs
 * reads closed days from this table and computes today live from
 * bot_reports. Composite PK + ON CONFLICT in the cron makes the
 * upsert idempotent across cron retries.
 */
export const botCostsDaily = pgTable(
  "bot_costs_daily",
  {
    botId: uuid("bot_id")
      .notNull()
      .references(() => users.id, { onDelete: "cascade" }),
    day: date("day").notNull(),
    usd: numeric("usd", { precision: 10, scale: 6 }).notNull().default("0"),
    reports: integer("reports").notNull().default(0),
    rolledUpAt: timestamp("rolled_up_at", { withTimezone: true })
      .notNull()
      .defaultNow(),
  },
  (t) => [
    primaryKey({ columns: [t.botId, t.day] }),
    index("idx_bot_costs_daily_day").on(t.day.desc()),
  ],
);
