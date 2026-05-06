/**
 * Submissions, tags, comments, votes, saves.
 *
 * These five tables travel together because the public reading
 * surface (feeds, threads, post pages) joins across them; splitting
 * by table would scatter the dependency map without a payoff.
 *
 * `submissions.score` is denormalized — maintained by the
 * `score_after_vote_change` trigger (see 0002_triggers.sql). Hot
 * rank is computed at query time via SQL expression in src/lib/rank.ts.
 */

import {
  index,
  integer,
  jsonb,
  pgTable,
  primaryKey,
  text,
  timestamp,
  uuid,
} from "drizzle-orm/pg-core";

import {
  contentStateEnum,
  submissionTypeEnum,
  submitterKindEnum,
} from "./enums";
import { users } from "./users";

export const submissions = pgTable(
  "submissions",
  {
    id: uuid("id").primaryKey().defaultRandom(),
    authorId: uuid("author_id")
      .notNull()
      .references(() => users.id),
    type: submissionTypeEnum("type").notNull(),
    title: text("title").notNull(),
    url: text("url"),
    text: text("text"),
    state: contentStateEnum("state").notNull().default("pending"),
    score: integer("score").notNull().default(0),
    readingTimeMin: integer("reading_time_min"),
    podcastMeta: jsonb("podcast_meta"),
    toolMeta: jsonb("tool_meta"),
    createdAt: timestamp("created_at", { withTimezone: true }).notNull().defaultNow(),
    publishedAt: timestamp("published_at", { withTimezone: true }),
    // Set only by post-window edits — see migration 0017. Within-
    // window edits stay silent (no bump). NULL = never edited (or
    // only within-window).
    updatedAt: timestamp("updated_at", { withTimezone: true }),
    deletedAt: timestamp("deleted_at", { withTimezone: true }),
    // Audit finding 3.3 — moderation lock/unlist columns.
    lockedAt: timestamp("locked_at", { withTimezone: true }),
    unlistedAt: timestamp("unlisted_at", { withTimezone: true }),
    // Added in 0008_editorial_runtime, repurposed in slice-2.
    // submitterKind: who created the submission (user / scout / import).
    // sourceId: trace identifier for non-user submissions. For
    // submitter_kind='scout' submitted via the public API/MCP (slice-2+),
    // this is the api_tokens.id UUID — joining to api_tokens recovers the
    // user, scopes, and prefix. For future editorial scouts driven by
    // editorial/sources.yml, this would be the source name; we'll add a
    // discriminator if/when both coexist. Null for organic user posts.
    submitterKind: submitterKindEnum("submitter_kind").notNull().default("user"),
    sourceId: text("source_id"),
  },
  (t) => [
    index("idx_submissions_state_created").on(t.state, t.createdAt.desc()),
    index("idx_submissions_state_score").on(t.state, t.score.desc()),
    index("idx_submissions_author").on(t.authorId),
    index("idx_submissions_source").on(t.sourceId),
  ],
);

export const tags = pgTable("tags", {
  slug: text("slug").primaryKey(),
  name: text("name").notNull(),
  tagline: text("tagline"),
  sortOrder: integer("sort_order").notNull().default(0),
});

export const submissionTags = pgTable(
  "submission_tags",
  {
    submissionId: uuid("submission_id")
      .notNull()
      .references(() => submissions.id, { onDelete: "cascade" }),
    tagSlug: text("tag_slug")
      .notNull()
      .references(() => tags.slug, { onDelete: "cascade" }),
  },
  (t) => [
    primaryKey({ columns: [t.submissionId, t.tagSlug] }),
    index("idx_submission_tags_tag").on(t.tagSlug, t.submissionId),
  ],
);

/**
 * Threaded via parent_id self-reference. Fetched with a recursive
 * CTE (see src/db/queries.ts).
 */
export const comments = pgTable(
  "comments",
  {
    id: uuid("id").primaryKey().defaultRandom(),
    authorId: uuid("author_id")
      .notNull()
      .references(() => users.id),
    submissionId: uuid("submission_id")
      .notNull()
      .references(() => submissions.id, { onDelete: "cascade" }),
    parentId: uuid("parent_id"),
    body: text("body").notNull(),
    state: contentStateEnum("state").notNull().default("approved"),
    score: integer("score").notNull().default(0),
    createdAt: timestamp("created_at", { withTimezone: true }).notNull().defaultNow(),
    // See migration 0017. Same semantics as submissions.updated_at.
    updatedAt: timestamp("updated_at", { withTimezone: true }),
    deletedAt: timestamp("deleted_at", { withTimezone: true }),
  },
  (t) => [
    index("idx_comments_submission_created").on(t.submissionId, t.createdAt),
    index("idx_comments_parent").on(t.parentId),
    index("idx_comments_author").on(t.authorId),
  ],
);

/**
 * Votes + saves use composite PKs on (user_id, submission_id) — one
 * row per (voter, submission). Vote flips update the same row.
 */
export const votes = pgTable(
  "votes",
  {
    userId: uuid("user_id")
      .notNull()
      .references(() => users.id, { onDelete: "cascade" }),
    submissionId: uuid("submission_id")
      .notNull()
      .references(() => submissions.id, { onDelete: "cascade" }),
    value: integer("value").notNull(),
    createdAt: timestamp("created_at", { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [primaryKey({ columns: [t.userId, t.submissionId] })],
);

export const saves = pgTable(
  "saves",
  {
    userId: uuid("user_id")
      .notNull()
      .references(() => users.id, { onDelete: "cascade" }),
    submissionId: uuid("submission_id")
      .notNull()
      .references(() => submissions.id, { onDelete: "cascade" }),
    createdAt: timestamp("created_at", { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [primaryKey({ columns: [t.userId, t.submissionId] })],
);
