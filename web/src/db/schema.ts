/**
 * Drizzle schema for claudepot.com v2.
 *
 * Single source of truth for the database structure. See
 * design/architecture.md §4 for the spec this implements.
 *
 * Editorial runtime tables (`decision_records`, `override_records`,
 * `scout_runs`) match `editorial/rubric.yml` v0.2.3 + `editorial/audits/
 * README.md`. Bot-side writes come from the `claudepot-office` private
 * repo running on mac-mini-home; reader-side reads come from the public
 * web app (this repo). See `editorial/transparency.md` for the
 * public-vs-private split.
 *
 * Migration 0008_editorial_runtime.sql replaced the v1 `ai_decisions` /
 * `moderation_overrides` scaffolding (no consumers existed) with these
 * richer per-criterion / per-persona tables.
 */

import {
  pgTable,
  pgEnum,
  uuid,
  text,
  integer,
  boolean,
  timestamp,
  jsonb,
  numeric,
  date,
  primaryKey,
  index,
  uniqueIndex,
  customType,
} from "drizzle-orm/pg-core";

/* ── citext custom type ─────────────────────────────────────────
 * Postgres `citext` is case-insensitive text. Used for usernames
 * and emails so "Ada" and "ada" collide on the unique index.
 * The `citext` extension must be enabled at DB-init time
 * (see migrations/0001_enable_citext.sql).
 */
const citext = customType<{ data: string; driverData: string }>({
  dataType: () => "citext",
});

/* ── Enums ──────────────────────────────────────────────────────
 * Naming: prefer noun-first (`user_role`) over verb-first.
 */

export const userRoleEnum = pgEnum("user_role", [
  "user",
  "staff",
  "locked",
  "system",
]);

export const submissionTypeEnum = pgEnum("submission_type", [
  "news",
  "tip",
  "tutorial",
  "course",
  "article",
  "podcast",
  "interview",
  "tool",
  "discussion",
  // Added in 0008_editorial_runtime — match editorial/rubric.yml v0.2.3 types.
  // The five v1 types above (tip, course, article) stay for backward compat
  // with seeded fixture data; new scout submissions use the v0.2.3 types only.
  "release",
  "paper",
  "workflow",
  "case_study",
  "prompt_pattern",
]);

// Editorial runtime enums (added in 0008_editorial_runtime).
export const submitterKindEnum = pgEnum("submitter_kind", [
  "user",
  "scout",
  "import",
]);

export const aiFinalDecisionEnum = pgEnum("ai_final_decision", [
  "accept",
  "reject",
  "borderline_to_human_queue",
]);

export const routingDestinationEnum = pgEnum("routing_destination", [
  "feed",
  "firehose",
  "human_queue",
]);

export const confidenceBandEnum = pgEnum("confidence_band", [
  "high",
  "low",
]);

export const contentStateEnum = pgEnum("content_state", [
  "pending",
  "approved",
  "rejected",
]);

export const flagStatusEnum = pgEnum("flag_status", ["open", "resolved"]);

export const notificationKindEnum = pgEnum("notification_kind", [
  "comment_reply",
  "submission_reply",
  "moderation",
  "mention",
]);

export const moderationActionEnum = pgEnum("moderation_action", [
  "lock",
  "unlist",
  "delete",
  "restore",
  "dismiss_flag",
  "lock_user",
  "approve",
  "reject",
  "delete_hard",
  // Tag-vocabulary governance — added in migration 0011 so /admin/flags
  // CRUD shows up alongside content moderation in /admin/log.
  "tag_create",
  "tag_rename",
  "tag_merge",
  "tag_retire",
]);

export const targetTypeEnum = pgEnum("target_type", [
  "submission",
  "comment",
]);

/* ── Users ──────────────────────────────────────────────────────
 * Auth.js DrizzleAdapter writes `name`, `email`, `emailVerified`,
 * `image` here (mapped via the adapter config in src/lib/auth.ts).
 * Our extra columns (username, role, karma, is_agent, bio) carry
 * domain semantics on top.
 */

export const users = pgTable(
  "users",
  {
    id: uuid("id").primaryKey().defaultRandom(),
    // Auth.js DrizzleAdapter writes these on OAuth signup:
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
    createdAt: timestamp("created_at", { withTimezone: true }).notNull().defaultNow(),
    updatedAt: timestamp("updated_at", { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [
    uniqueIndex("idx_users_username").on(t.username),
    uniqueIndex("idx_users_email").on(t.email),
  ],
);

/* ── Auth.js standard tables ────────────────────────────────────
 * Managed by @auth/drizzle-adapter. Do not modify column names.
 */

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
  (t) => [
    primaryKey({ columns: [t.provider, t.providerAccountId] }),
  ],
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

/* ── Submissions + tags ─────────────────────────────────────────
 * `score` is denormalized — maintained by `score_after_vote_change`
 * trigger (see 0002_triggers.sql). Hot rank computed at query time
 * via SQL expression in src/lib/rank.ts.
 */

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

/* ── Comments ───────────────────────────────────────────────────
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
    deletedAt: timestamp("deleted_at", { withTimezone: true }),
  },
  (t) => [
    index("idx_comments_submission_created").on(t.submissionId, t.createdAt),
    index("idx_comments_parent").on(t.parentId),
    index("idx_comments_author").on(t.authorId),
  ],
);

/* ── Votes + saves ──────────────────────────────────────────────
 * Composite PKs on (user_id, submission_id) — one row per
 * (voter, submission). Vote flips update the same row.
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

/* ── Flags ──────────────────────────────────────────────────────
 * Polymorphic `target_type` + `target_id`; not FK-enforced (typical
 * tradeoff for polymorphic refs). Filter integrity in app layer.
 */

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

/* ── Notifications ──────────────────────────────────────────────
 * Pull-based inbox. `payload` jsonb shape varies by `kind`.
 */

export const notifications = pgTable(
  "notifications",
  {
    id: uuid("id").primaryKey().defaultRandom(),
    userId: uuid("user_id")
      .notNull()
      .references(() => users.id, { onDelete: "cascade" }),
    kind: notificationKindEnum("kind").notNull(),
    payload: jsonb("payload").notNull(),
    readAt: timestamp("read_at", { withTimezone: true }),
    createdAt: timestamp("created_at", { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [
    index("idx_notifications_user_unread").on(t.userId, t.createdAt.desc()),
  ],
);

/* ── Per-user lists ─────────────────────────────────────────────
 * Hidden submissions and muted tags. Used as filters in feed reads.
 */

export const userHiddenSubmissions = pgTable(
  "user_hidden_submissions",
  {
    userId: uuid("user_id")
      .notNull()
      .references(() => users.id, { onDelete: "cascade" }),
    submissionId: uuid("submission_id")
      .notNull()
      .references(() => submissions.id, { onDelete: "cascade" }),
    hiddenAt: timestamp("hidden_at", { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [primaryKey({ columns: [t.userId, t.submissionId] })],
);

export const userTagMutes = pgTable(
  "user_tag_mutes",
  {
    userId: uuid("user_id")
      .notNull()
      .references(() => users.id, { onDelete: "cascade" }),
    tagSlug: text("tag_slug")
      .notNull()
      .references(() => tags.slug, { onDelete: "cascade" }),
    mutedAt: timestamp("muted_at", { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [primaryKey({ columns: [t.userId, t.tagSlug] })],
);

export const userEmailPrefs = pgTable("user_email_prefs", {
  userId: uuid("user_id")
    .primaryKey()
    .references(() => users.id, { onDelete: "cascade" }),
  digestWeekly: boolean("digest_weekly").notNull().default(true),
  notifyReplies: boolean("notify_replies").notNull().default(true),
  updatedAt: timestamp("updated_at", { withTimezone: true }).notNull().defaultNow(),
});

/* ── Digest sends ───────────────────────────────────────────────
 * Idempotency guard for the weekly digest cron. One row per
 * (user, ISO-week). The cron does INSERT ... ON CONFLICT DO NOTHING
 * RETURNING user_id and only emails recipients whose insert produced
 * a row. This makes the cron safe to retry: a rerun in the same week
 * cannot deliver duplicate digests.
 */

export const digestSends = pgTable(
  "digest_sends",
  {
    userId: uuid("user_id")
      .notNull()
      .references(() => users.id, { onDelete: "cascade" }),
    // ISO-8601 week key, e.g. "2026-W18". Text not date so retries
    // across the Sun→Mon midnight boundary still collapse onto the
    // same row.
    weekKey: text("week_key").notNull(),
    sentAt: timestamp("sent_at", { withTimezone: true })
      .notNull()
      .defaultNow(),
  },
  (t) => [primaryKey({ columns: [t.userId, t.weekKey] })],
);

/* ── Moderation log (public) ────────────────────────────────────
 * Append-only. Every staff action creates a row. Visible at
 * /admin/log to any authed user.
 */

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

/* ── Editorial runtime (added in 0008_editorial_runtime) ────────
 * Replaces the v1 ai_decisions / moderation_overrides scaffolding
 * (no consumers) with the richer per-criterion / per-persona shape
 * that matches `editorial/rubric.yml` v0.2.3.
 *
 * Bot-side writers live in the `claudepot-office` private repo on
 * mac-mini-home; reader-side readers live in this repo. See
 * `editorial/transparency.md` for the privacy split.
 */

export const decisionRecords = pgTable(
  "decision_records",
  {
    id: uuid("id").primaryKey().defaultRandom(),
    submissionId: uuid("submission_id")
      .notNull()
      .references(() => submissions.id, { onDelete: "cascade" }),
    rubricVersion: text("rubric_version").notNull(),
    audienceDocVersion: text("audience_doc_version").notNull(),
    appliedPersona: text("applied_persona").notNull(), // open enum — new personas land without migration
    perCriterionScores: jsonb("per_criterion_scores").notNull(),
    weightedTotal: numeric("weighted_total", { precision: 8, scale: 3 }).notNull(),
    hardRejectsHit: jsonb("hard_rejects_hit").notNull().default([]),
    inclusionGates: jsonb("inclusion_gates").notNull(),
    typeInferred: submissionTypeEnum("type_inferred").notNull(),
    subSegmentInferred: text("sub_segment_inferred").notNull(),
    confidence: confidenceBandEnum("confidence").notNull(),
    oneLineWhy: text("one_line_why").notNull(),
    finalDecision: aiFinalDecisionEnum("final_decision").notNull(),
    routing: routingDestinationEnum("routing").notNull(),
    modelId: text("model_id").notNull(),
    promptHash: text("prompt_hash"),
    costUsd: numeric("cost_usd", { precision: 10, scale: 6 }),
    scoredAt: timestamp("scored_at", { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [
    index("idx_decision_records_submission").on(t.submissionId),
    index("idx_decision_records_routing").on(t.routing, t.scoredAt.desc()),
    index("idx_decision_records_persona").on(t.appliedPersona, t.scoredAt.desc()),
    index("idx_decision_records_rubric_version").on(t.rubricVersion),
  ],
);

export const overrideRecords = pgTable(
  "override_records",
  {
    id: uuid("id").primaryKey().defaultRandom(),
    decisionRecordId: uuid("decision_record_id")
      .notNull()
      .references(() => decisionRecords.id, { onDelete: "cascade" }),
    reviewerId: uuid("reviewer_id")
      .notNull()
      .references(() => users.id),
    originalDecision: aiFinalDecisionEnum("original_decision").notNull(),
    overrideDecision: aiFinalDecisionEnum("override_decision").notNull(),
    overrideRouting: routingDestinationEnum("override_routing").notNull(),
    reviewerScores: jsonb("reviewer_scores"), // optional per-criterion re-score
    reason: text("reason").notNull(),
    createdAt: timestamp("created_at", { withTimezone: true }).notNull().defaultNow(),
  },
  (t) => [
    index("idx_override_records_decision").on(t.decisionRecordId),
    index("idx_override_records_reviewer").on(t.reviewerId, t.createdAt.desc()),
    index("idx_override_records_created").on(t.createdAt.desc()),
  ],
);

/* scout_runs — one row per scout invocation per source. Aggregated
 * counts surface on /office/sources; per-source rules stay private
 * inside the claudepot-office repo. */
export const scoutRuns = pgTable(
  "scout_runs",
  {
    id: uuid("id").primaryKey().defaultRandom(),
    sourceId: text("source_id").notNull(), // matches editorial sources.yml id
    startedAt: timestamp("started_at", { withTimezone: true }).notNull(),
    finishedAt: timestamp("finished_at", { withTimezone: true }).notNull(),
    itemsPulled: integer("items_pulled").notNull().default(0),
    itemsKept: integer("items_kept").notNull().default(0),
    itemsDropped: integer("items_dropped").notNull().default(0),
    error: text("error"),
  },
  (t) => [
    index("idx_scout_runs_source_started").on(t.sourceId, t.startedAt.desc()),
    index("idx_scout_runs_started").on(t.startedAt.desc()),
  ],
);

/* ── Projects (minimal in v1) ───────────────────────────────────
 * Curated collections of submissions.
 */

export const projects = pgTable("projects", {
  id: uuid("id").primaryKey().defaultRandom(),
  slug: text("slug").notNull().unique(),
  name: text("name").notNull(),
  blurb: text("blurb"),
  ownerId: uuid("owner_id")
    .notNull()
    .references(() => users.id),
  // GitHub-sourced metadata (migration 0005). Refresh via
  // `pnpm tsx scripts/sync-projects.ts`.
  repoUrl: text("repo_url"),
  siteUrl: text("site_url"),
  primaryLanguage: text("primary_language"),
  stars: integer("stars").notNull().default(0),
  updatedAt: timestamp("updated_at", { withTimezone: true }),
  // README snapshot + editorial lede (migration 0010). README is
  // pulled from GitHub via the sync script; editorial is hand-authored.
  readmeMd: text("readme_md"),
  editorialMd: text("editorial_md"),
  createdAt: timestamp("created_at", { withTimezone: true }).notNull().defaultNow(),
});

export const projectSubmissions = pgTable(
  "project_submissions",
  {
    projectId: uuid("project_id")
      .notNull()
      .references(() => projects.id, { onDelete: "cascade" }),
    submissionId: uuid("submission_id")
      .notNull()
      .references(() => submissions.id, { onDelete: "cascade" }),
  },
  (t) => [primaryKey({ columns: [t.projectId, t.submissionId] })],
);

// Project ↔ tag join (migration 0010). Mirrors `submission_tags`
// so the related-feed query can JOIN projects → project_tags →
// submission_tags → submissions cleanly.
export const projectTags = pgTable(
  "project_tags",
  {
    projectId: uuid("project_id")
      .notNull()
      .references(() => projects.id, { onDelete: "cascade" }),
    tagSlug: text("tag_slug")
      .notNull()
      .references(() => tags.slug, { onDelete: "cascade" }),
  },
  (t) => [
    primaryKey({ columns: [t.projectId, t.tagSlug] }),
    index("idx_project_tags_tag").on(t.tagSlug, t.projectId),
  ],
);

/* ── Daily metrics rollup ───────────────────────────────────────
 * Populated by /api/cron/daily-rollup.
 */

export const metricsDaily = pgTable("metrics_daily", {
  day: date("day").primaryKey(),
  submissionsTotal: integer("submissions_total").notNull().default(0),
  commentsTotal: integer("comments_total").notNull().default(0),
  votesTotal: integer("votes_total").notNull().default(0),
  signupsTotal: integer("signups_total").notNull().default(0),
  activeUsers24h: integer("active_users_24h").notNull().default(0),
});

/* ── Public API tokens ──────────────────────────────────────────
 * Per-user Personal Access Tokens for the public REST + MCP API.
 * Plaintext (`shn_pat_<28 random url-safe-base64 chars>`) is shown
 * once at creation; only the SHA-256 hex digest is stored.
 *
 * Scopes are an open text array, validated in app code (see
 * src/lib/api/scopes.ts) — same pattern as decision_records.applied_persona.
 * New scopes land without a migration.
 *
 * Default expiry: 180 days from creation (set in app code, NOT in DB,
 * so staff can opt out of expiry per token). `revoked_at IS NULL` and
 * (`expires_at IS NULL OR expires_at > now()`) are the active checks.
 */

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

/* ── API token usage (daily buckets, for rate limits) ──────────
 * One row per (token, UTC date). Counters are incremented atomically
 * via INSERT … ON CONFLICT DO UPDATE in src/lib/api/rate-limit.ts.
 * Pruning is NOT yet wired — table grows at one row per
 * (active_token, day). Add a dedicated cron route when volume
 * warrants (target: keep last 90 days).
 */

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
  },
  (t) => [
    primaryKey({ columns: [t.tokenId, t.bucketDate] }),
  ],
);

/* ── API token audit events ────────────────────────────────────
 * Lifecycle log for PATs (mint, revoke, future: expired,
 * rate_limited). Kept separate from moderation_log because that
 * table records staff actions on content; PAT events are usually
 * user actions on their own tokens. token_id is SET NULL on token
 * delete so the audit row survives.
 */

export const apiTokenEventEnum = pgEnum("api_token_event", [
  "mint",
  "revoke",
]);

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
