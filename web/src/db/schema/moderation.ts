/**
 * flags + moderation_log.
 *
 *   - flags: polymorphic `target_type` + `target_id`; not FK-enforced
 *     (typical tradeoff for polymorphic refs). Filter integrity in
 *     app layer.
 *   - moderation_log: append-only. Every state-changing staff action
 *     creates a row. AI auto-rejects also write here under the
 *     policy-moderator system user. Visible at /admin/log to any
 *     authed user — that page is a public transparency surface, not
 *     staff-only.
 */

import {
  index,
  pgTable,
  text,
  timestamp,
  uuid,
} from "drizzle-orm/pg-core";

import {
  flagStatusEnum,
  moderationActionEnum,
  targetTypeEnum,
} from "./enums";
import { users } from "./users";

export const flags = pgTable(
  "flags",
  {
    id: uuid("id").primaryKey().defaultRandom(),
    reporterId: uuid("reporter_id")
      .notNull()
      .references(() => users.id),
    targetType: targetTypeEnum("target_type").notNull(),
    targetId: uuid("target_id").notNull(),
    reason: text("reason").notNull(),
    status: flagStatusEnum("status").notNull().default("open"),
    resolvedBy: uuid("resolved_by").references(() => users.id),
    createdAt: timestamp("created_at", { withTimezone: true }).notNull().defaultNow(),
    resolvedAt: timestamp("resolved_at", { withTimezone: true }),
  },
  (t) => [
    index("idx_flags_open").on(t.targetType, t.targetId, t.status),
    index("idx_flags_reporter").on(t.reporterId),
  ],
);

export const moderationLog = pgTable(
  "moderation_log",
  {
    id: uuid("id").primaryKey().defaultRandom(),
    staffId: uuid("staff_id")
      .notNull()
      .references(() => users.id),
    action: moderationActionEnum("action").notNull(),
    targetType: targetTypeEnum("target_type"),
    targetId: uuid("target_id"),
    note: text("note"),
    createdAt: timestamp("created_at", { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [
    index("idx_moderation_log_created").on(t.createdAt.desc()),
    index("idx_moderation_log_staff").on(t.staffId),
  ],
);
