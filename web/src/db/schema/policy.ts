/**
 * Policy moderator (migration 0018).
 *
 * One row per `moderate()` call regardless of verdict. Distinct
 * from `decision_records` (editorial taste): policy lives in this
 * repo, taste in `claudepot-office`. See dev-docs/policy-moderator-plan.md.
 *
 * `author_id` is required (the verdict is always about an author);
 * `target_id` is nullable so the illegal-comment block path can
 * write a row without a comment ever being inserted.
 */

import {
  index,
  numeric,
  pgTable,
  smallint,
  text,
  timestamp,
  uuid,
} from "drizzle-orm/pg-core";

import { targetTypeEnum } from "./enums";
import { users } from "./users";

export const policyDecisions = pgTable(
  "policy_decisions",
  {
    id: uuid("id").primaryKey().defaultRandom(),
    authorId: uuid("author_id")
      .notNull()
      .references(() => users.id),
    targetType: targetTypeEnum("target_type").notNull(),
    targetId: uuid("target_id"),
    // Open enums kept as text so new categories or verdicts can land
    // without a migration. App code (lib/moderation/types.ts) is the
    // authoritative whitelist.
    verdict: text("verdict").notNull(),       // 'pass' | 'reject'
    category: text("category"),               // 'spam' | 'abuse' | 'illegal' | 'doxxing' | 'off_topic' | NULL on pass
    confidence: text("confidence").notNull(), // 'high' | 'low'
    oneLineWhy: text("one_line_why").notNull(),
    modelId: text("model_id").notNull(),
    promptVersion: text("prompt_version").notNull(),
    costUsd: numeric("cost_usd", { precision: 10, scale: 6 }),
    passNumber: smallint("pass_number").notNull().default(1),
    decidedAt: timestamp("decided_at", { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [
    index("idx_policy_decisions_target").on(t.targetType, t.targetId, t.decidedAt.desc()),
    index("idx_policy_decisions_author_created").on(t.authorId, t.decidedAt.desc()),
    index("idx_policy_decisions_category_created").on(t.category, t.decidedAt.desc()),
  ],
);
