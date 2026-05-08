/**
 * Submissions, tags, comments, votes, saves.
 *
 * These five tables travel together because the public reading
 * surface (feeds, threads, post pages) joins across them; splitting
 * by table would scatter the dependency map without a payoff.
 *
 * `submissions.score` is denormalized — maintained by the
 * `fn_submission_score_after_vote` trigger (see 0001_triggers.sql).
 * Hot rank is computed at query time via the inlined SQL expression
 * `HOT_RANK_EXPR` in src/db/queries.ts.
 */

import {
  boolean,
  customType,
  index,
  integer,
  jsonb,
  pgTable,
  primaryKey,
  text,
  timestamp,
  uuid,
} from "drizzle-orm/pg-core";
import { sql } from "drizzle-orm";

import {
  contentStateEnum,
  submissionTypeEnum,
  submitterKindEnum,
} from "./enums";
import { users } from "./users";

/**
 * tsvector custom type for the FTS column on submissions. The
 * actual column DDL is `tsvector GENERATED ALWAYS AS (…) STORED`
 * (see migration 0003_fts.sql) — Drizzle has no first-class
 * support for generated columns, so we declare the column here
 * with `tsvector` only and rely on the migration's GENERATED
 * clause for population.
 *
 * Without this declaration, `drizzle-kit push` sees a column that
 * exists in the DB but not in the schema and DROPS it (which
 * happened on 2026-05-06 when push was used to apply migrations
 * 0018–0021 — search_vec went with it). Declaring it here keeps
 * push's diff happy.
 *
 * If push ever tries to "fix" this column by dropping the GENERATED
 * clause, that's a bigger problem — switch to migrate / psql for
 * production schema changes.
 */
const tsvector = customType<{ data: string; driverData: string }>({
  dataType: () => "tsvector",
});

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
    // Full-text search column — `tsvector GENERATED ALWAYS AS (…)
    // STORED` per migration 0003_fts.sql. Declared here as `tsvector`
    // only so drizzle-kit push doesn't see a phantom column to drop.
    // The GENERATED clause is owned by the migration, not the schema.
    searchVec: tsvector("search_vec"),
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
  // Migration 0022 — Ada-proposed tags land with pending_review=true
  // and stay hidden from the public /c catalog until staff approves
  // them at /admin/tags. Staff approval flips this to false.
  pendingReview: boolean("pending_review").notNull().default(false),
});

/**
 * Provenance values accepted by submission_tags.source. The
 * matching CHECK constraint lives in migration 0022 — keep these
 * two in sync if you add a third source (e.g. 'import').
 */
export const SUBMISSION_TAG_SOURCES = ["ai", "user"] as const;
export type SubmissionTagSource = (typeof SUBMISSION_TAG_SOURCES)[number];

export const submissionTags = pgTable(
  "submission_tags",
  {
    submissionId: uuid("submission_id")
      .notNull()
      .references(() => submissions.id, { onDelete: "cascade" }),
    tagSlug: text("tag_slug")
      .notNull()
      .references(() => tags.slug, { onDelete: "cascade" }),
    // Migration 0022 — provenance. 'user' for tags from the submit
    // form's tags[]; 'ai' for tags Ada applied during moderation.
    // CHECK constraint in the migration enforces the value space at
    // the DB layer; .$type() narrows TS so writes that pass an
    // unknown string fail at compile time before the constraint
    // ever fires. The set of valid values is exported as
    // `SUBMISSION_TAG_SOURCES` so call sites can assert on the union.
    source: text("source")
      .notNull()
      .default("user")
      .$type<SubmissionTagSource>(),
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
    // Migration 0036 — bot↔bot replies set is_meta=true so they
    // drop out of public engagement counters. The comment still
    // renders in the thread; only the metric-side joins exclude
    // it. Office side sets this; humans never can.
    isMeta: boolean("is_meta").notNull().default(false),
  },
  (t) => [
    index("idx_comments_submission_created").on(t.submissionId, t.createdAt),
    index("idx_comments_parent").on(t.parentId),
    index("idx_comments_author").on(t.authorId),
    // Migration 0031 — partial covering index for the public-feed
    // /u/[username] Comments tab + /api/v1/users/[username]/comments.
    // Both filter on (author_id, state='approved', deleted_at IS NULL)
    // and order by created_at DESC.
    index("idx_comments_author_visible_created")
      .on(t.authorId, t.createdAt.desc())
      .where(sql`${t.state} = 'approved' AND ${t.deletedAt} IS NULL`),
    // Migration 0036 — counter-side covering index that filters
    // out bot↔bot replies. Used by the public commentCount
    // aggregate; the unfiltered idx_comments_submission_created
    // above still serves the thread-rendering query that *does*
    // include is_meta=true rows.
    index("idx_comments_submission_visible_nonmeta")
      .on(t.submissionId, t.createdAt)
      .where(
        sql`${t.state} = 'approved' AND ${t.deletedAt} IS NULL AND ${t.isMeta} = false`,
      ),
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
