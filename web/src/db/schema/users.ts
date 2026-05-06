/**
 * users + Auth.js standard tables.
 *
 * Auth.js DrizzleAdapter writes `name`, `email`, `emailVerified`,
 * `image` here (mapped via the adapter config in src/lib/auth.ts).
 * Our extra columns (username, role, karma, is_agent, bio) carry
 * domain semantics on top.
 *
 * Auth.js standard tables (accounts, sessions, verificationTokens)
 * are managed by @auth/drizzle-adapter — do not modify column names.
 */

import {
  boolean,
  integer,
  pgTable,
  primaryKey,
  text,
  timestamp,
  uniqueIndex,
  uuid,
} from "drizzle-orm/pg-core";

import { citext, userRoleEnum } from "./enums";

export const users = pgTable(
  "users",
  {
    id: uuid("id").primaryKey().defaultRandom(),
    name: text("name"),
    email: citext("email").notNull(),
    emailVerified: timestamp("email_verified", { withTimezone: true, mode: "date" }),
    image: text("image"),
    // Our extended fields. On OAuth signup we mirror name → username
    // and image → avatar_url in src/lib/auth.ts events.createUser.
    username: citext("username").notNull(),
    // Self-rename tracking — see canSelfRename + SELF_RENAME_* in
    // src/lib/username.ts. After the grace window or count is
    // exhausted, only admins can change the username.
    usernameLastChangedAt: timestamp("username_last_changed_at", {
      withTimezone: true,
    }),
    selfUsernameRenameCount: integer("self_username_rename_count")
      .notNull()
      .default(0),
    avatarUrl: text("avatar_url"),
    bio: text("bio"),
    role: userRoleEnum("role").notNull().default("user"),
    isAgent: boolean("is_agent").notNull().default(false),
    karma: integer("karma").notNull().default(0),
    // Per-bot exemption from the AI policy moderator. Only meaningful
    // when isAgent=true; staff/system roles already skip the gate via
    // role check. Toggled at /admin/users; see lib/moderation/exempt.ts.
    botModerationExempt: boolean("bot_moderation_exempt").notNull().default(false),
    createdAt: timestamp("created_at", { withTimezone: true }).notNull().defaultNow(),
    updatedAt: timestamp("updated_at", { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [
    uniqueIndex("idx_users_username").on(t.username),
    uniqueIndex("idx_users_email").on(t.email),
  ],
);

export const accounts = pgTable(
  "accounts",
  {
    userId: uuid("user_id")
      .notNull()
      .references(() => users.id, { onDelete: "cascade" }),
    type: text("type").notNull(),
    provider: text("provider").notNull(),
    providerAccountId: text("provider_account_id").notNull(),
    refresh_token: text("refresh_token"),
    access_token: text("access_token"),
    expires_at: integer("expires_at"),
    token_type: text("token_type"),
    scope: text("scope"),
    id_token: text("id_token"),
    session_state: text("session_state"),
  },
  (t) => [primaryKey({ columns: [t.provider, t.providerAccountId] })],
);

export const sessions = pgTable("sessions", {
  sessionToken: text("session_token").primaryKey(),
  userId: uuid("user_id")
    .notNull()
    .references(() => users.id, { onDelete: "cascade" }),
  expires: timestamp("expires", { mode: "date", withTimezone: true }).notNull(),
});

export const verificationTokens = pgTable(
  "verification_tokens",
  {
    identifier: text("identifier").notNull(),
    token: text("token").notNull(),
    expires: timestamp("expires", { mode: "date", withTimezone: true }).notNull(),
  },
  (t) => [primaryKey({ columns: [t.identifier, t.token] })],
);
