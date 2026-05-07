/**
 * Public API tokens — Per-user Personal Access Tokens for the public
 * REST + MCP API.
 *
 * Plaintext (`cdp_pat_<28 random url-safe-base64 chars>`) is shown
 * once at creation; only the SHA-256 hex digest is stored.
 *
 * Scopes are an open text array, validated in app code (see
 * src/lib/api/scopes.ts) — same pattern as decision_records.applied_persona.
 * New scopes land without a migration.
 *
 * Default expiry: 180 days from creation (set in app code, NOT in DB,
 * so staff can opt out of expiry per token). `revoked_at IS NULL` and
 * (`expires_at IS NULL OR expires_at > now()`) are the active checks.
 *
 * Usage table (api_token_usage) is one row per (token, UTC date),
 * incremented atomically via INSERT … ON CONFLICT DO UPDATE in
 * src/lib/api/rate-limit.ts. Pruning is NOT yet wired — table grows
 * at one row per (active_token, day). Add a dedicated cron route
 * when volume warrants (target: keep last 90 days).
 *
 * Events table (api_token_events) is the lifecycle log — kept
 * separate from moderation_log because that table records staff
 * actions on content; PAT events are usually user actions on their
 * own tokens. token_id is SET NULL on token delete so the audit row
 * survives.
 */

import {
  date,
  index,
  integer,
  jsonb,
  pgTable,
  primaryKey,
  text,
  timestamp,
  uniqueIndex,
  uuid,
} from "drizzle-orm/pg-core";

import { apiTokenEventEnum } from "./enums";
import { users } from "./users";

export const apiTokens = pgTable(
  "api_tokens",
  {
    id: uuid("id").primaryKey().defaultRandom(),
    userId: uuid("user_id")
      .notNull()
      .references(() => users.id, { onDelete: "cascade" }),
    name: text("name").notNull(),
    displayPrefix: text("display_prefix").notNull(),
    hashedSecret: text("hashed_secret").notNull(),
    scopes: text("scopes").array().notNull().default([]),
    lastUsedAt: timestamp("last_used_at", { withTimezone: true }),
    expiresAt: timestamp("expires_at", { withTimezone: true }),
    revokedAt: timestamp("revoked_at", { withTimezone: true }),
    createdAt: timestamp("created_at", { withTimezone: true })
      .notNull()
      .defaultNow(),
  },
  (t) => [
    uniqueIndex("idx_api_tokens_hashed_secret").on(t.hashedSecret),
    index("idx_api_tokens_user").on(t.userId, t.createdAt.desc()),
  ],
);

export const apiTokenUsage = pgTable(
  "api_token_usage",
  {
    tokenId: uuid("token_id")
      .notNull()
      .references(() => apiTokens.id, { onDelete: "cascade" }),
    bucketDate: date("bucket_date").notNull(),
    submissionsCount: integer("submissions_count").notNull().default(0),
    commentsCount: integer("comments_count").notNull().default(0),
    votesCount: integer("votes_count").notNull().default(0),
    savesCount: integer("saves_count").notNull().default(0),
    readsCount: integer("reads_count").notNull().default(0),
    // Migration 0025_bot_reports — separate bucket for bot
    // self-reporting calls so a chatty bot doesn't burn a token's
    // submission/comment quota.
    botsCount: integer("bots_count").notNull().default(0),
  },
  (t) => [primaryKey({ columns: [t.tokenId, t.bucketDate] })],
);

export const apiTokenEvents = pgTable(
  "api_token_events",
  {
    id: uuid("id").primaryKey().defaultRandom(),
    tokenId: uuid("token_id").references(() => apiTokens.id, {
      onDelete: "set null",
    }),
    userId: uuid("user_id")
      .notNull()
      .references(() => users.id, { onDelete: "cascade" }),
    event: apiTokenEventEnum("event").notNull(),
    scopes: text("scopes").array(),
    metadata: jsonb("metadata"),
    occurredAt: timestamp("occurred_at", { withTimezone: true })
      .notNull()
      .defaultNow(),
  },
  (t) => [
    index("idx_api_token_events_user").on(t.userId, t.occurredAt.desc()),
    index("idx_api_token_events_token").on(t.tokenId, t.occurredAt.desc()),
  ],
);
