/**
 * DB-backed system prompt for the AI policy moderator (migration 0021).
 *
 * One row per saved version. Exactly one row may have active=true at
 * any time, enforced by a partial unique index. Activation flips
 * happen in a single transaction so the index never observes two
 * active rows.
 *
 * The hardcoded fallback in web/src/lib/moderation/prompt.ts is the
 * boot-time default; once a row exists here with active=true,
 * lib/moderation/prompt-store.ts returns it instead. Empty table
 * keeps the fallback in effect — fresh deploys work without staff
 * intervention.
 */

import {
  boolean,
  index,
  pgTable,
  text,
  timestamp,
  uniqueIndex,
  uuid,
} from "drizzle-orm/pg-core";
import { sql } from "drizzle-orm";

import { users } from "./users";

export const moderationPrompts = pgTable(
  "moderation_prompts",
  {
    id: uuid("id").primaryKey().defaultRandom(),
    version: text("version").notNull().unique(),
    systemPrompt: text("system_prompt").notNull(),
    active: boolean("active").notNull().default(false),
    createdBy: uuid("created_by")
      .notNull()
      .references(() => users.id),
    createdAt: timestamp("created_at", { withTimezone: true })
      .notNull()
      .defaultNow(),
    note: text("note"),
  },
  (t) => [
    uniqueIndex("idx_moderation_prompts_active")
      .on(t.active)
      .where(sql`${t.active} = true`),
    index("idx_moderation_prompts_created").on(t.createdAt.desc()),
  ],
);
