/**
 * DB-backed query functions. Mirrors the public surface of
 * src/lib/prototype-fixtures.ts so pages can swap their imports
 * without changing how they consume the data.
 *
 * Return types are intentionally identical to the prototype's
 * `Submission`, `CommentNode`, etc. — fields the v2 schema doesn't
 * carry (`domain`, `subjects`, separate `upvotes`/`downvotes`) are
 * derived or synthesized from `score`. Real upvote/downvote split
 * can come back when we denormalize those columns; until then,
 * `upvotes = max(score, 0)`, `downvotes = max(-score, 0)`.
 */

import { and, desc, eq, gte, isNull, notInArray, sql } from "drizzle-orm";

import { db } from "./client";
import { encodeCursor, type CursorTime } from "@/lib/api/cursor";
import {
  comments,
  decisionRecords,
  projectTags,
  projects,
  saves,
  submissionTags,
  submissions,
  tags,
  userTagFollows,
  userTagMutes,
  users,
  votes,
} from "./schema";

/* ── Per-user mute filter ──────────────────────────────────────── */

async function getMutedTagSlugs(userId: string | null): Promise<string[]> {
  if (!userId) return [];
  const rows = await db
    .select({ tagSlug: userTagMutes.tagSlug })
    .from(userTagMutes)
    .where(eq(userTagMutes.userId, userId));
  return rows.map((r) => r.tagSlug);
}

// Postgres SQLSTATE 42P01 — relation does not exist. Thrown when
// migration 0026 has been merged into the codebase but not yet
// applied to the target database (the rules/db-migrations.md flow
// requires manual `psql` apply for prod). Treat as "feature not
// available" rather than 500-ing the page.
function isUndefinedTable(err: unknown): boolean {
  return (
    typeof err === "object" &&
    err !== null &&
    "code" in err &&
    (err as { code?: unknown }).code === "42P01"
  );
}

/**
 * Whether `userId` follows `tagSlug`. Returns false for null/anon
 * viewers without a query — used by the FollowTagButton to decide
 * which label/state to render.
 *
 * Returns false (not throws) if the user_tag_follows table doesn't
 * exist yet — see isUndefinedTable comment above.
 */
export async function isTagFollowed(
  userId: string | null,
  tagSlug: string,
): Promise<boolean> {
  if (!userId) return false;
  try {
    const [row] = await db
      .select({ tagSlug: userTagFollows.tagSlug })
      .from(userTagFollows)
      .where(
        and(eq(userTagFollows.userId, userId), eq(userTagFollows.tagSlug, tagSlug)),
      )
      .limit(1);
    return Boolean(row);
  } catch (err) {
    if (isUndefinedTable(err)) return false;
    throw err;
  }
}

/**
 * SQL fragment that excludes submissions tagged with any slug in `muted`.
 * Returns a no-op TRUE expression when there's nothing to mute.
 */
function notInMutedTags(muted: string[]) {
  if (muted.length === 0) return sql`TRUE`;
  return notInArray(
    submissions.id,
    db
      .select({ id: submissionTags.submissionId })
      .from(submissionTags)
      .where(inArrayLiteral(submissionTags.tagSlug, muted)),
  );
}

// Helper: drizzle-orm exposes `inArray` via the same import path; keeping the
// indirection so we can swap the impl if it becomes inefficient.
function inArrayLiteral(col: typeof submissionTags.tagSlug, vals: string[]) {
  return sql`${col} IN (${sql.join(vals.map((v) => sql`${v}`), sql`, `)})`;
}

import type {
  CommentNode,
  Project,
  Submission,
  Tag,
  User,
} from "@/lib/prototype-fixtures";

import { deriveDomain } from "@/lib/url";

/* ── Internal helpers ───────────────────────────────────────────── */

function synthesizeVotes(score: number): { upvotes: number; downvotes: number } {
  return score >= 0
    ? { upvotes: score, downvotes: 0 }
    : { upvotes: 0, downvotes: -score };
}

/* ── Row → public shape mappers ─────────────────────────────────── */

type SubmissionRowJoined = {
  id: string;
  type: Submission["type"];
  effectiveType: string;
  title: string;
  url: string | null;
  text: string | null;
  state: "pending" | "approved" | "rejected" | "draft";
  score: number;
  readingTimeMin: number | null;
  podcastMeta: unknown;
  toolMeta: unknown;
  createdAt: Date;
  publishedAt: Date | null;
  updatedAt: Date | null;
  authorUsername: string;
  authorImageUrl: string | null;
  authorIsAgent: boolean;
  commentsCount: number;
  tagSlugs: string[];
};

function mapSubmission(r: SubmissionRowJoined): Submission {
  const { upvotes, downvotes } = synthesizeVotes(r.score);
  // The DB enforces that type_inferred is a valid submission_type, so
  // effectiveType (when present) is always a valid SubmissionType.
  // Surface it only when it actually differs from the bot's claim —
  // otherwise the field is noise.
  const effectiveType =
    r.effectiveType && r.effectiveType !== r.type
      ? (r.effectiveType as Submission["type"])
      : undefined;
  return {
    id: r.id,
    user: r.authorUsername,
    user_image_url: r.authorImageUrl,
    type: r.type,
    effective_type: effectiveType,
    tags: r.tagSlugs,
    title: r.title,
    url: r.url,
    domain: deriveDomain(r.url) ?? "",
    // `subjects` is a legacy field on the prototype's Submission type
    // (concept-first IA, superseded by tag-based). Kept empty for type
    // compatibility with components; will drop when the type is trimmed.
    subjects: [],
    upvotes,
    downvotes,
    comments: r.commentsCount,
    submitted_at: r.createdAt.toISOString(),
    updated_at: r.updatedAt?.toISOString(),
    text: r.text ?? undefined,
    auto_posted: r.authorIsAgent || undefined,
    reading_time_min: r.readingTimeMin ?? undefined,
    tool_meta:
      (r.toolMeta as Submission["tool_meta"] | null) ?? undefined,
    podcast_meta:
      (r.podcastMeta as Submission["podcast_meta"] | null) ?? undefined,
    state: r.state,
  };
}

function mapUser(r: typeof users.$inferSelect): User {
  return {
    username: r.username,
    display_name: r.name ?? r.username,
    karma: r.karma,
    joined: r.createdAt.toISOString().slice(0, 10),
    bio: r.bio ?? "",
    provider: "email",
    is_system: r.role === "system" || undefined,
    image_url: r.image ?? r.avatarUrl ?? null,
  };
}

/* ── Submission feed queries ────────────────────────────────────── */

// Audit finding 3.3 — feeds exclude unlisted submissions. The permalink
// (getSubmissionById) does NOT filter on this; staff-unlisted content
// stays accessible by direct URL.
const FEED_BASE_FILTERS = () =>
  and(
    eq(submissions.state, "approved"),
    isNull(submissions.deletedAt),
    isNull(submissions.unlistedAt),
  );


const SUBMISSION_BASE_SELECT = {
  id: submissions.id,
  type: submissions.type,
  // Effective type — the office's editorial-mesh classification
  // (decision_records.type_inferred) takes precedence over the bot's
  // initial submissions.type when an editorial decision exists.
  // Gated on author.is_agent=true so citizen submissions render
  // their own claimed type (the office never overrides citizen
  // metadata; that's a boundary discipline call from
  // 2026-05-08-polity-api-replies.md). When a bot row has multiple
  // decisions, the most-recent (scoredAt DESC, id DESC) wins —
  // same precedence as the override surface.
  effectiveType: sql<string>`(
    CASE WHEN ${users.isAgent} THEN
      COALESCE(
        (SELECT ${decisionRecords.typeInferred}::text
         FROM ${decisionRecords}
         WHERE ${decisionRecords.submissionId} = ${submissions.id}
         ORDER BY ${decisionRecords.scoredAt} DESC, ${decisionRecords.id} DESC
         LIMIT 1),
        ${submissions.type}::text
      )
    ELSE ${submissions.type}::text END
  )`,
  title: submissions.title,
  url: submissions.url,
  text: submissions.text,
  state: submissions.state,
  score: submissions.score,
  readingTimeMin: submissions.readingTimeMin,
  podcastMeta: submissions.podcastMeta,
  toolMeta: submissions.toolMeta,
  createdAt: submissions.createdAt,
  publishedAt: submissions.publishedAt,
  updatedAt: submissions.updatedAt,
  authorUsername: users.username,
  authorImageUrl: users.image,
  authorIsAgent: users.isAgent,
  // Public count must not leak moderation activity: only count
  // approved, non-deleted comments. Mirrors lib/api/queries.ts.
  // Also excludes is_meta=true (migration 0036) so bot↔bot replies
  // don't inflate the public engagement signal — the comments still
  // render in the thread, but the count drops them.
  commentsCount: sql<number>`(
    SELECT COUNT(*)::int FROM ${comments}
    WHERE ${comments.submissionId} = ${submissions.id}
      AND ${comments.state} = 'approved'
      AND ${comments.deletedAt} IS NULL
      AND ${comments.isMeta} = false
  )`,
  // Migration 0022 — exclude pending_review=true tags from public
  // submission rows. Without this filter, an Ada-proposed tag still
  // awaiting staff review would appear as a chip on /c rows linking
  // to a /c/<slug> page that 404s (getTagBySlug filters pending too).
  // Joining tags via the slug FK and gating on pending_review keeps
  // the chip and the landing page in sync.
  tagSlugs: sql<string[]>`ARRAY(
    SELECT ${submissionTags.tagSlug}
    FROM ${submissionTags}
    INNER JOIN ${tags} ON ${tags.slug} = ${submissionTags.tagSlug}
    WHERE ${submissionTags.submissionId} = ${submissions.id}
      AND ${tags.pendingReview} = false
  )`,
};

// GREATEST(..., 0) clamps the age so future-dated fixture rows don't
// produce a negative denominator (POWER(neg, 1.8) is a complex result
// in Postgres and errors out — code 2201F).
//
// Migration 0039 — ranking consumes score_human only. Bot votes feed
// score_bot for the reach metric (rendered separately on submission
// detail and the office dashboard) but never drive what humans see
// in the feed. This is the load-bearing abuse-mitigation that lets
// citizen-bots exist safely. See web/dev-docs/citizen-bots.md.
const HOT_RANK_EXPR = sql<number>`(
  POWER(GREATEST(${submissions.scoreHuman} - 1, 0), 0.8) /
  POWER(GREATEST(EXTRACT(EPOCH FROM (NOW() - ${submissions.createdAt})) / 3600, 0) + 2, 1.8)
)`;

// Sitemap protocol allows up to 50,000 URLs per file; we cap well
// below that to bound memory and Vercel function time. If the corpus
// grows past this, split into a sitemap index instead of raising the
// cap.
const SITEMAP_MAX_SUBMISSIONS = 10_000;

export async function getAllSubmissions(
  limit: number = SITEMAP_MAX_SUBMISSIONS,
): Promise<Submission[]> {
  // Use the same public-visibility predicate as feed reads — sitemap
  // entries are crawler-visible URLs, so anything unlisted, deleted,
  // or unapproved must NOT be enumerated. Without unlistedAt the
  // unlist moderator action would be a no-op for SEO. See FEED_BASE
  // _FILTERS for the canonical predicate.
  const rows = await db
    .select(SUBMISSION_BASE_SELECT)
    .from(submissions)
    .innerJoin(users, eq(users.id, submissions.authorId))
    .where(FEED_BASE_FILTERS())
    .orderBy(desc(submissions.createdAt))
    .limit(limit);
  return rows.map(mapSubmission);
}

// Cap the size of any caller-requested feed slice. 200 is generous
// for any real surface (current callers ask for 25–30); the cap stops
// a buggy/hostile call site from issuing an unbounded read.
const MAX_FEED_LIMIT = 200;

function clampLimit(limit: number | undefined): number | null {
  if (limit === undefined) return null;
  if (!Number.isFinite(limit) || limit <= 0) return null;
  return Math.min(Math.floor(limit), MAX_FEED_LIMIT);
}

export async function getSubmissionsByHot(
  viewerId: string | null = null,
  limit?: number,
): Promise<Submission[]> {
  const muted = await getMutedTagSlugs(viewerId);
  const cap = clampLimit(limit);
  const q = db
    .select(SUBMISSION_BASE_SELECT)
    .from(submissions)
    .innerJoin(users, eq(users.id, submissions.authorId))
    .where(and(FEED_BASE_FILTERS(), notInMutedTags(muted)))
    .orderBy(desc(HOT_RANK_EXPR));
  const rows = cap ? await q.limit(cap) : await q;
  return rows.map(mapSubmission);
}

/**
 * Cursor pagination for time-ordered feeds. The cursor encodes the
 * tail row's (createdAt, id) so the next call resumes after it.
 * Compatible with the `lib/api/cursor.ts` time-cursor shape so reader
 * pages and the v1 API can read each other's cursors.
 */
export interface Page<T> {
  items: T[];
  nextCursor: string | null;
}

const DEFAULT_PAGE_LIMIT = 30;

export async function getSubmissionsByNew({
  viewerId = null,
  limit = DEFAULT_PAGE_LIMIT,
  cursor = null,
}: {
  viewerId?: string | null;
  limit?: number;
  cursor?: CursorTime | null;
} = {}): Promise<Page<Submission>> {
  const muted = await getMutedTagSlugs(viewerId);
  const cap = clampLimit(limit) ?? DEFAULT_PAGE_LIMIT;
  const cond = [FEED_BASE_FILTERS(), notInMutedTags(muted)];
  if (cursor) {
    const cutoff = new Date(cursor.t);
    cond.push(
      sql`(${submissions.createdAt}, ${submissions.id}) < (${cutoff}, ${cursor.id})`,
    );
  }
  const rows = await db
    .select(SUBMISSION_BASE_SELECT)
    .from(submissions)
    .innerJoin(users, eq(users.id, submissions.authorId))
    .where(and(...cond))
    .orderBy(desc(submissions.createdAt), desc(submissions.id))
    .limit(cap + 1);
  const hasMore = rows.length > cap;
  const slice = hasMore ? rows.slice(0, cap) : rows;
  const tail = slice[slice.length - 1];
  const nextCursor =
    hasMore && tail
      ? encodeCursor({ t: tail.createdAt.getTime(), id: tail.id })
      : null;
  return { items: slice.map(mapSubmission), nextCursor };
}

/**
 * /top is a leaderboard, not a queue. Capped at TOP_FEED_LIMIT rows
 * per range with no pagination — the audit's mutable-score-cursor
 * concern (votes shifting a row across the cursor between page-1
 * and page-N reads, causing skip/duplicate) doesn't apply when
 * there's no cursor. Users wanting more depth use /new (immutable
 * createdAt cursor) or filter by tag.
 *
 * Matches the HN /top model: small, fresh, single-page.
 */
const TOP_FEED_LIMIT = 30;

export async function getSubmissionsByTop({
  range = "day",
  viewerId = null,
}: {
  range?: "day" | "week" | "all";
  viewerId?: string | null;
} = {}): Promise<Submission[]> {
  const muted = await getMutedTagSlugs(viewerId);
  const cutoff =
    range === "all"
      ? null
      : new Date(
          Date.now() - (range === "day" ? 1 : 7) * 86_400_000,
        );
  const cond = [FEED_BASE_FILTERS(), notInMutedTags(muted)];
  if (cutoff) cond.push(gte(submissions.createdAt, cutoff));
  const rows = await db
    .select(SUBMISSION_BASE_SELECT)
    .from(submissions)
    .innerJoin(users, eq(users.id, submissions.authorId))
    .where(and(...cond))
    // Migration 0039 — top feed ranks by human score, same rationale
    // as HOT_RANK_EXPR.
    .orderBy(desc(submissions.scoreHuman), desc(submissions.id))
    .limit(TOP_FEED_LIMIT);
  return rows.map(mapSubmission);
}

export async function getSubmissionById(
  id: string,
): Promise<Submission | undefined> {
  if (!isUuid(id)) return undefined;
  const [row] = await db
    .select(SUBMISSION_BASE_SELECT)
    .from(submissions)
    .innerJoin(users, eq(users.id, submissions.authorId))
    .where(eq(submissions.id, id))
    .limit(1);
  return row ? mapSubmission(row) : undefined;
}

export async function getSubmissionsByUser(
  username: string,
  opts: {
    includeAll?: boolean;
    limit?: number;
    cursor?: CursorTime | null;
  } = {},
): Promise<Page<Submission>> {
  const { includeAll = false, limit = DEFAULT_PAGE_LIMIT, cursor = null } = opts;
  const cap = clampLimit(limit) ?? DEFAULT_PAGE_LIMIT;
  const cond = [eq(users.username, username), isNull(submissions.deletedAt)];
  if (!includeAll) {
    // `includeAll=true` is the author's own pending/rejected view; it
    // intentionally bypasses approval + unlist gates. Public callers
    // (profile page, RSS) pass false and need the same predicate as
    // the rest of the feed surfaces.
    cond.push(eq(submissions.state, "approved"));
    cond.push(isNull(submissions.unlistedAt));
  }
  if (cursor) {
    const cutoff = new Date(cursor.t);
    cond.push(
      sql`(${submissions.createdAt}, ${submissions.id}) < (${cutoff}, ${cursor.id})`,
    );
  }
  const rows = await db
    .select(SUBMISSION_BASE_SELECT)
    .from(submissions)
    .innerJoin(users, eq(users.id, submissions.authorId))
    .where(and(...cond))
    .orderBy(desc(submissions.createdAt), desc(submissions.id))
    .limit(cap + 1);
  const hasMore = rows.length > cap;
  const slice = hasMore ? rows.slice(0, cap) : rows;
  const tail = slice[slice.length - 1];
  const nextCursor =
    hasMore && tail
      ? encodeCursor({ t: tail.createdAt.getTime(), id: tail.id })
      : null;
  return { items: slice.map(mapSubmission), nextCursor };
}

export async function getPendingForUser(
  username: string,
  opts: { limit?: number; cursor?: CursorTime | null } = {},
): Promise<Page<Submission>> {
  const { limit = DEFAULT_PAGE_LIMIT, cursor = null } = opts;
  const cap = clampLimit(limit) ?? DEFAULT_PAGE_LIMIT;
  const cond = [
    eq(users.username, username),
    sql`${submissions.state} != 'approved'`,
    isNull(submissions.deletedAt),
  ];
  if (cursor) {
    const cutoff = new Date(cursor.t);
    cond.push(
      sql`(${submissions.createdAt}, ${submissions.id}) < (${cutoff}, ${cursor.id})`,
    );
  }
  const rows = await db
    .select(SUBMISSION_BASE_SELECT)
    .from(submissions)
    .innerJoin(users, eq(users.id, submissions.authorId))
    .where(and(...cond))
    .orderBy(desc(submissions.createdAt), desc(submissions.id))
    .limit(cap + 1);
  const hasMore = rows.length > cap;
  const slice = hasMore ? rows.slice(0, cap) : rows;
  const tail = slice[slice.length - 1];
  const nextCursor =
    hasMore && tail
      ? encodeCursor({ t: tail.createdAt.getTime(), id: tail.id })
      : null;
  return { items: slice.map(mapSubmission), nextCursor };
}

/* ── Comments ───────────────────────────────────────────────────── */

// buildCommentTree + CommentRow live in lib/comments/tree.ts so the
// pure-function test can import them without triggering the Neon
// client initialization.
import { buildCommentTree, type CommentRow } from "@/lib/comments/tree";

export { buildCommentTree, type CommentRow };

// Audit finding 6.1 — bound the comment fetch so a viral post (1000+
// comments) doesn't unbounded-scan. 200 covers the long tail; pagination
// for deeper threads is a follow-up.
const COMMENT_FETCH_LIMIT = 200;

async function fetchCommentsRows(submissionId: string): Promise<CommentRow[]> {
  const rows = await db
    .select({
      id: comments.id,
      parentId: comments.parentId,
      body: comments.body,
      state: comments.state,
      score: comments.score,
      createdAt: comments.createdAt,
      updatedAt: comments.updatedAt,
      authorUsername: users.username,
      authorImageUrl: users.image,
      deletedAt: comments.deletedAt,
    })
    .from(comments)
    .innerJoin(users, eq(users.id, comments.authorId))
    .where(eq(comments.submissionId, submissionId))
    .orderBy(desc(comments.score))
    .limit(COMMENT_FETCH_LIMIT);
  // Comments share the content_state enum with submissions, so the
  // inferred union now includes 'draft' (added in 0036 for submissions
  // only — comments never enter that state). Narrow with a cast.
  return rows as CommentRow[];
}

export async function getCommentsForSubmission(
  id: string,
): Promise<CommentNode[]> {
  if (!isUuid(id)) return [];
  const rows = await fetchCommentsRows(id);
  return buildCommentTree(rows, /* publicOnly */ true);
}

/* ── Users ──────────────────────────────────────────────────────── */

export async function getUser(username: string): Promise<User | undefined> {
  const [u] = await db
    .select()
    .from(users)
    .where(eq(users.username, username))
    .limit(1);
  return u ? mapUser(u) : undefined;
}

export interface UserCommentSummary {
  id: string;
  submissionId: string;
  submissionTitle: string;
  body: string;
  score: number;
  submitted_at: string;
}

/**
 * Public-visible comments authored by `username`, newest first. Excludes
 * tombstoned (deleted/rejected) comments and any comment whose parent
 * submission is itself unlisted, deleted, or unapproved — those would
 * otherwise leak through to the profile page.
 */
export async function getCommentsByUser(
  username: string,
  opts: { limit?: number; cursor?: CursorTime | null } = {},
): Promise<Page<UserCommentSummary>> {
  const { limit = DEFAULT_PAGE_LIMIT, cursor = null } = opts;
  const cap = clampLimit(limit) ?? DEFAULT_PAGE_LIMIT;

  const [u] = await db
    .select({ id: users.id })
    .from(users)
    .where(eq(users.username, username))
    .limit(1);
  if (!u) return { items: [], nextCursor: null };

  const cond = [
    eq(comments.authorId, u.id),
    eq(comments.state, "approved"),
    isNull(comments.deletedAt),
    eq(submissions.state, "approved"),
    isNull(submissions.deletedAt),
    isNull(submissions.unlistedAt),
  ];
  if (cursor) {
    const cutoff = new Date(cursor.t);
    cond.push(
      sql`(${comments.createdAt}, ${comments.id}) < (${cutoff}, ${cursor.id})`,
    );
  }

  const rows = await db
    .select({
      id: comments.id,
      submissionId: comments.submissionId,
      submissionTitle: submissions.title,
      body: comments.body,
      score: comments.score,
      createdAt: comments.createdAt,
    })
    .from(comments)
    .innerJoin(submissions, eq(submissions.id, comments.submissionId))
    .where(and(...cond))
    .orderBy(desc(comments.createdAt), desc(comments.id))
    .limit(cap + 1);

  const hasMore = rows.length > cap;
  const slice = hasMore ? rows.slice(0, cap) : rows;
  const tail = slice[slice.length - 1];
  const nextCursor =
    hasMore && tail
      ? encodeCursor({ t: tail.createdAt.getTime(), id: tail.id })
      : null;

  return {
    items: slice.map((r) => ({
      id: r.id,
      submissionId: r.submissionId,
      submissionTitle: r.submissionTitle,
      body: r.body,
      score: r.score,
      submitted_at: r.createdAt.toISOString(),
    })),
    nextCursor,
  };
}

/* ── Saved + upvoted (per-user lists) ──────────────────────────── */

export async function getSavedForUser(
  username: string,
  opts: { limit?: number; cursor?: CursorTime | null } = {},
): Promise<Page<Submission>> {
  const { limit = DEFAULT_PAGE_LIMIT, cursor = null } = opts;
  const cap = clampLimit(limit) ?? DEFAULT_PAGE_LIMIT;
  const [u] = await db
    .select({ id: users.id })
    .from(users)
    .where(eq(users.username, username))
    .limit(1);
  if (!u) return { items: [], nextCursor: null };
  const cond = [
    eq(saves.userId, u.id),
    isNull(submissions.deletedAt),
    eq(submissions.state, "approved"),
  ];
  if (cursor) {
    const cutoff = new Date(cursor.t);
    cond.push(
      sql`(${saves.createdAt}, ${submissions.id}) < (${cutoff}, ${cursor.id})`,
    );
  }
  const rows = await db
    .select({ ...SUBMISSION_BASE_SELECT, savedAt: saves.createdAt })
    .from(saves)
    .innerJoin(submissions, eq(submissions.id, saves.submissionId))
    .innerJoin(users, eq(users.id, submissions.authorId))
    .where(and(...cond))
    .orderBy(desc(saves.createdAt), desc(submissions.id))
    .limit(cap + 1);
  const hasMore = rows.length > cap;
  const slice = hasMore ? rows.slice(0, cap) : rows;
  const tail = slice[slice.length - 1];
  const nextCursor =
    hasMore && tail
      ? encodeCursor({ t: tail.savedAt.getTime(), id: tail.id })
      : null;
  return { items: slice.map(mapSubmission), nextCursor };
}

export async function getUpvotedByUser(
  username: string,
  opts: { limit?: number; cursor?: CursorTime | null } = {},
): Promise<Page<Submission>> {
  const { limit = DEFAULT_PAGE_LIMIT, cursor = null } = opts;
  const cap = clampLimit(limit) ?? DEFAULT_PAGE_LIMIT;
  const [u] = await db
    .select({ id: users.id })
    .from(users)
    .where(eq(users.username, username))
    .limit(1);
  if (!u) return { items: [], nextCursor: null };
  const cond = [
    eq(votes.userId, u.id),
    eq(votes.value, 1),
    isNull(submissions.deletedAt),
    eq(submissions.state, "approved"),
  ];
  if (cursor) {
    const cutoff = new Date(cursor.t);
    cond.push(
      sql`(${votes.createdAt}, ${submissions.id}) < (${cutoff}, ${cursor.id})`,
    );
  }
  const rows = await db
    .select({ ...SUBMISSION_BASE_SELECT, votedAt: votes.createdAt })
    .from(votes)
    .innerJoin(submissions, eq(submissions.id, votes.submissionId))
    .innerJoin(users, eq(users.id, submissions.authorId))
    .where(and(...cond))
    .orderBy(desc(votes.createdAt), desc(submissions.id))
    .limit(cap + 1);
  const hasMore = rows.length > cap;
  const slice = hasMore ? rows.slice(0, cap) : rows;
  const tail = slice[slice.length - 1];
  const nextCursor =
    hasMore && tail
      ? encodeCursor({ t: tail.votedAt.getTime(), id: tail.id })
      : null;
  return { items: slice.map(mapSubmission), nextCursor };
}

/* ── Tags ───────────────────────────────────────────────────────── */

export async function getAllTags(): Promise<Tag[]> {
  // Migration 0022 — pending_review=true tags are Ada-proposed and
  // awaiting staff approval. Hide them from the public catalog so
  // the /c index only shows curated tags. Staff sees and approves
  // them at /admin/tags.
  const rows = await db
    .select()
    .from(tags)
    .where(eq(tags.pendingReview, false))
    .orderBy(tags.sortOrder);
  return rows.map((t) => ({
    slug: t.slug,
    name: t.name,
    tagline: t.tagline ?? "",
  }));
}

export async function getTagBySlug(slug: string): Promise<Tag | undefined> {
  // Per migration 0022, a pending_review tag has no public landing
  // page — return undefined so /c/<slug> 404s until staff approves
  // it. Submissions that already link to a pending tag remain in
  // the DB; they're just not surfaced by tag.
  const [t] = await db
    .select()
    .from(tags)
    .where(and(eq(tags.slug, slug), eq(tags.pendingReview, false)))
    .limit(1);
  return t ? { slug: t.slug, name: t.name, tagline: t.tagline ?? "" } : undefined;
}

export async function getTopTags(): Promise<Array<Tag & { count: number }>> {
  const cutoff = new Date(Date.now() - 7 * 86_400_000);
  const rows = await db
    .select({
      slug: tags.slug,
      name: tags.name,
      tagline: tags.tagline,
      count: sql<number>`COUNT(${submissions.id})::int`,
    })
    .from(tags)
    .leftJoin(submissionTags, eq(submissionTags.tagSlug, tags.slug))
    .leftJoin(
      submissions,
      and(
        eq(submissions.id, submissionTags.submissionId),
        eq(submissions.state, "approved"),
        isNull(submissions.deletedAt),
        // Match the public-feed predicate; without unlistedAt the
        // top-tag count counts hidden submissions and the rail
        // ranks tags whose hidden volume is high.
        isNull(submissions.unlistedAt),
        gte(submissions.createdAt, cutoff),
      ),
    )
    // Migration 0022 — drop pending_review=true rows so the home
    // page top-tags rail only shows staff-approved tags.
    .where(eq(tags.pendingReview, false))
    .groupBy(tags.slug, tags.name, tags.tagline, tags.sortOrder)
    // Count submissions.id (NULL on tag rows with no joined visible
    // submission) instead of submission_tags.submissionId, which
    // would count rows that pre-leftJoin point at filtered-out
    // submissions and inflate the rank.
    .orderBy(desc(sql`COUNT(${submissions.id})`));
  return rows.map((r) => ({
    slug: r.slug,
    name: r.name,
    tagline: r.tagline ?? "",
    count: Number(r.count ?? 0),
  }));
}

export async function getSubmissionsByTag(
  slug: string,
  opts: {
    viewerId?: string | null;
    limit?: number;
    cursor?: CursorTime | null;
  } = {},
): Promise<Page<Submission>> {
  const { viewerId = null, limit = DEFAULT_PAGE_LIMIT, cursor = null } = opts;
  const cap = clampLimit(limit) ?? DEFAULT_PAGE_LIMIT;
  const muted = await getMutedTagSlugs(viewerId);
  const cond = [
    eq(submissionTags.tagSlug, slug),
    FEED_BASE_FILTERS(),
    notInMutedTags(muted),
  ];
  if (cursor) {
    const cutoff = new Date(cursor.t);
    cond.push(
      sql`(${submissions.createdAt}, ${submissions.id}) < (${cutoff}, ${cursor.id})`,
    );
  }
  // Tag pages used to order by HOT_RANK_EXPR. Switched to createdAt
  // for stable cursor pagination — hot rank decays over time, which
  // makes a (rank, id) cursor unstable across reads. createdAt-desc
  // is a common feed shape for tag/topic pages and aligns with how
  // /new behaves elsewhere in the reader.
  const rows = await db
    .select(SUBMISSION_BASE_SELECT)
    .from(submissions)
    .innerJoin(users, eq(users.id, submissions.authorId))
    .innerJoin(submissionTags, eq(submissionTags.submissionId, submissions.id))
    .where(and(...cond))
    .orderBy(desc(submissions.createdAt), desc(submissions.id))
    .limit(cap + 1);
  const hasMore = rows.length > cap;
  const slice = hasMore ? rows.slice(0, cap) : rows;
  const tail = slice[slice.length - 1];
  const nextCursor =
    hasMore && tail
      ? encodeCursor({ t: tail.createdAt.getTime(), id: tail.id })
      : null;
  return { items: slice.map(mapSubmission), nextCursor };
}

/* ── Projects ───────────────────────────────────────────────────── */

export async function getAllProjects(): Promise<Project[]> {
  const rows = await db.select().from(projects).orderBy(projects.name);
  return rows.map(mapProject);
}

export async function getProjectBySlug(slug: string): Promise<Project | undefined> {
  const [p] = await db
    .select()
    .from(projects)
    .where(eq(projects.slug, slug))
    .limit(1);
  return p ? mapProject(p) : undefined;
}

function mapProject(p: typeof projects.$inferSelect): Project {
  return {
    slug: p.slug,
    name: p.name,
    tagline: p.blurb ?? "",
    repo_url: p.repoUrl ?? "",
    site_url: p.siteUrl ?? null,
    primary_language: p.primaryLanguage,
    stars: p.stars,
    updated_at: p.updatedAt?.toISOString(),
    readme_md: p.readmeMd,
    editorial_md: p.editorialMd,
  };
}

/**
 * Tags bound to a project (migration 0010). Order: tags.sort_order,
 * then name. Returns [] when no tags are bound — the detail page reads
 * this as "show empty state" rather than calling /c/* chips.
 */
export async function getProjectTags(slug: string): Promise<Tag[]> {
  const rows = await db
    .select({
      slug: tags.slug,
      name: tags.name,
      tagline: tags.tagline,
      sortOrder: tags.sortOrder,
    })
    .from(tags)
    .innerJoin(projectTags, eq(projectTags.tagSlug, tags.slug))
    .innerJoin(projects, eq(projects.id, projectTags.projectId))
    .where(eq(projects.slug, slug))
    .orderBy(tags.sortOrder, tags.name);
  return rows.map((r) => ({
    slug: r.slug,
    name: r.name,
    tagline: r.tagline ?? "",
  }));
}

/**
 * Submissions whose tags overlap any tag bound to this project. Replaces
 * the prior ILIKE-on-title/url match (audit finding 5.1.b) — the new
 * join is honest about the relationship and uses real indexes.
 *
 * Empty when the project has no tags bound yet (xiaolai populates
 * `design/fixtures/project-tags.json` to wire up the relationship).
 */
export async function getRelatedSubmissionsForProject(
  slug: string,
  limit = 4,
): Promise<Submission[]> {
  const rows = await db
    .select(SUBMISSION_BASE_SELECT)
    .from(submissions)
    .innerJoin(users, eq(users.id, submissions.authorId))
    .where(
      and(
        FEED_BASE_FILTERS(),
        sql`EXISTS (
          SELECT 1 FROM ${submissionTags}
          INNER JOIN ${projectTags} ON ${projectTags.tagSlug} = ${submissionTags.tagSlug}
          INNER JOIN ${projects}     ON ${projects.id}        = ${projectTags.projectId}
          WHERE ${submissionTags.submissionId} = ${submissions.id}
            AND ${projects.slug} = ${slug}
        )`,
      ),
    )
    .orderBy(desc(submissions.createdAt))
    .limit(limit);
  return rows.map(mapSubmission);
}

/* ── Helpers ────────────────────────────────────────────────────── */

const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

function isUuid(s: string): boolean {
  return UUID_RE.test(s);
}
