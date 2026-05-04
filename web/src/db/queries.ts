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
import {
  comments,
  projectTags,
  projects,
  saves,
  submissionTags,
  submissions,
  tags,
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

/* ── Internal helpers ───────────────────────────────────────────── */

function deriveDomain(url: string | null | undefined): string {
  if (!url) return "sha.com";
  try {
    return new URL(url).hostname;
  } catch {
    return "";
  }
}

function synthesizeVotes(score: number): { upvotes: number; downvotes: number } {
  return score >= 0
    ? { upvotes: score, downvotes: 0 }
    : { upvotes: 0, downvotes: -score };
}

/* ── Row → public shape mappers ─────────────────────────────────── */

type SubmissionRowJoined = {
  id: string;
  type: Submission["type"];
  title: string;
  url: string | null;
  text: string | null;
  state: "pending" | "approved" | "rejected";
  score: number;
  readingTimeMin: number | null;
  podcastMeta: unknown;
  toolMeta: unknown;
  createdAt: Date;
  publishedAt: Date | null;
  authorUsername: string;
  authorImageUrl: string | null;
  authorIsAgent: boolean;
  commentsCount: number;
  tagSlugs: string[];
};

function mapSubmission(r: SubmissionRowJoined): Submission {
  const { upvotes, downvotes } = synthesizeVotes(r.score);
  return {
    id: r.id,
    user: r.authorUsername,
    user_image_url: r.authorImageUrl,
    type: r.type,
    tags: r.tagSlugs,
    title: r.title,
    url: r.url,
    domain: deriveDomain(r.url),
    // `subjects` is a legacy field on the prototype's Submission type
    // (concept-first IA, superseded by tag-based). Kept empty for type
    // compatibility with components; will drop when the type is trimmed.
    subjects: [],
    upvotes,
    downvotes,
    comments: r.commentsCount,
    submitted_at: r.createdAt.toISOString(),
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
  authorUsername: users.username,
  authorImageUrl: users.image,
  authorIsAgent: users.isAgent,
  commentsCount: sql<number>`(SELECT COUNT(*)::int FROM ${comments} WHERE ${comments.submissionId} = ${submissions.id} AND ${comments.deletedAt} IS NULL)`,
  tagSlugs: sql<string[]>`ARRAY(SELECT ${submissionTags.tagSlug} FROM ${submissionTags} WHERE ${submissionTags.submissionId} = ${submissions.id})`,
};

// GREATEST(..., 0) clamps the age so future-dated fixture rows don't
// produce a negative denominator (POWER(neg, 1.8) is a complex result
// in Postgres and errors out — code 2201F).
const HOT_RANK_EXPR = sql<number>`(
  POWER(GREATEST(${submissions.score} - 1, 0), 0.8) /
  POWER(GREATEST(EXTRACT(EPOCH FROM (NOW() - ${submissions.createdAt})) / 3600, 0) + 2, 1.8)
)`;

export async function getAllSubmissions(): Promise<Submission[]> {
  const rows = await db
    .select(SUBMISSION_BASE_SELECT)
    .from(submissions)
    .innerJoin(users, eq(users.id, submissions.authorId))
    .where(isNull(submissions.deletedAt))
    .orderBy(desc(submissions.createdAt));
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

export async function getSubmissionsByNew(
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
    .orderBy(desc(submissions.createdAt));
  const rows = cap ? await q.limit(cap) : await q;
  return rows.map(mapSubmission);
}

export async function getSubmissionsByTop(
  range: "day" | "week" | "all" = "day",
  viewerId: string | null = null,
  limit?: number,
): Promise<Submission[]> {
  const muted = await getMutedTagSlugs(viewerId);
  const cutoff =
    range === "all"
      ? null
      : new Date(
          Date.now() - (range === "day" ? 1 : 7) * 86_400_000,
        );
  const where = cutoff
    ? and(FEED_BASE_FILTERS(), notInMutedTags(muted), gte(submissions.createdAt, cutoff))
    : and(FEED_BASE_FILTERS(), notInMutedTags(muted));
  const cap = clampLimit(limit);
  const q = db
    .select(SUBMISSION_BASE_SELECT)
    .from(submissions)
    .innerJoin(users, eq(users.id, submissions.authorId))
    .where(where)
    .orderBy(desc(submissions.score));
  const rows = cap ? await q.limit(cap) : await q;
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
  includeAll = false,
): Promise<Submission[]> {
  const baseConds = [eq(users.username, username), isNull(submissions.deletedAt)];
  if (!includeAll) baseConds.push(eq(submissions.state, "approved"));
  const rows = await db
    .select(SUBMISSION_BASE_SELECT)
    .from(submissions)
    .innerJoin(users, eq(users.id, submissions.authorId))
    .where(and(...baseConds))
    .orderBy(desc(submissions.createdAt));
  return rows.map(mapSubmission);
}

export async function getPendingForUser(username: string): Promise<Submission[]> {
  const rows = await db
    .select(SUBMISSION_BASE_SELECT)
    .from(submissions)
    .innerJoin(users, eq(users.id, submissions.authorId))
    .where(
      and(
        eq(users.username, username),
        sql`${submissions.state} != 'approved'`,
        isNull(submissions.deletedAt),
      ),
    )
    .orderBy(desc(submissions.createdAt));
  return rows.map(mapSubmission);
}

/* ── Comments ───────────────────────────────────────────────────── */

type CommentRow = {
  id: string;
  parentId: string | null;
  body: string;
  state: "pending" | "approved" | "rejected";
  score: number;
  createdAt: Date;
  authorUsername: string;
  authorImageUrl: string | null;
  deletedAt: Date | null;
};

function buildCommentTree(
  rows: CommentRow[],
  publicOnly: boolean,
): CommentNode[] {
  // Audit finding 3.1 — preserve thread structure when a parent is
  // filtered (rejected) but has approved descendants. Build the full
  // tree first, then prune tombstone leaves (filtered/deleted nodes
  // with no visible children).
  const byParent = new Map<string | null, CommentRow[]>();
  for (const r of rows) {
    const list = byParent.get(r.parentId) ?? [];
    list.push(r);
    byParent.set(r.parentId, list);
  }

  function buildLevel(parentId: string | null): CommentNode[] {
    const kids = byParent.get(parentId) ?? [];
    return kids
      .map((r): CommentNode | null => {
        const children = buildLevel(r.id);
        const filtered = publicOnly && r.state !== "approved";
        const tombstoned = r.deletedAt != null || filtered;
        // Prune tombstone leaves; surface tombstone branches with kids.
        if (tombstoned && children.length === 0) return null;
        const { upvotes, downvotes } = synthesizeVotes(r.score);
        return {
          id: r.id,
          user: tombstoned ? "[deleted]" : r.authorUsername,
          submitted_at: r.createdAt.toISOString(),
          upvotes: tombstoned ? 0 : upvotes,
          downvotes: tombstoned ? 0 : downvotes,
          // Body is still scrubbed to "[deleted]" so a leak via a
          // forgotten consumer doesn't expose the original text. The
          // `tombstoned` flag below is what the renderer keys off,
          // independent of the body's literal content.
          body: tombstoned ? "[deleted]" : r.body,
          children,
          state: r.state,
          tombstoned,
        };
      })
      .filter((n): n is CommentNode => n !== null);
  }
  return buildLevel(null);
}

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
      authorUsername: users.username,
      authorImageUrl: users.image,
      deletedAt: comments.deletedAt,
    })
    .from(comments)
    .innerJoin(users, eq(users.id, comments.authorId))
    .where(eq(comments.submissionId, submissionId))
    .orderBy(desc(comments.score))
    .limit(COMMENT_FETCH_LIMIT);
  return rows;
}

export async function getCommentsForSubmission(
  id: string,
): Promise<CommentNode[]> {
  if (!isUuid(id)) return [];
  const rows = await fetchCommentsRows(id);
  return buildCommentTree(rows, /* publicOnly */ true);
}

export async function getAllCommentsForSubmission(
  id: string,
): Promise<CommentNode[]> {
  if (!isUuid(id)) return [];
  const rows = await fetchCommentsRows(id);
  return buildCommentTree(rows, /* publicOnly */ false);
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

export async function getAllUsers(): Promise<User[]> {
  const rows = await db.select().from(users).orderBy(desc(users.karma));
  return rows.map(mapUser);
}

/* ── Saved + upvoted (per-user lists) ──────────────────────────── */

export async function getSavedForUser(username: string): Promise<Submission[]> {
  const [u] = await db
    .select({ id: users.id })
    .from(users)
    .where(eq(users.username, username))
    .limit(1);
  if (!u) return [];
  const rows = await db
    .select(SUBMISSION_BASE_SELECT)
    .from(saves)
    .innerJoin(submissions, eq(submissions.id, saves.submissionId))
    .innerJoin(users, eq(users.id, submissions.authorId))
    .where(
      and(
        eq(saves.userId, u.id),
        isNull(submissions.deletedAt),
        eq(submissions.state, "approved"),
      ),
    )
    .orderBy(desc(saves.createdAt));
  return rows.map(mapSubmission);
}

export async function getUpvotedByUser(username: string): Promise<Submission[]> {
  const [u] = await db
    .select({ id: users.id })
    .from(users)
    .where(eq(users.username, username))
    .limit(1);
  if (!u) return [];
  const rows = await db
    .select(SUBMISSION_BASE_SELECT)
    .from(votes)
    .innerJoin(submissions, eq(submissions.id, votes.submissionId))
    .innerJoin(users, eq(users.id, submissions.authorId))
    .where(
      and(
        eq(votes.userId, u.id),
        eq(votes.value, 1),
        isNull(submissions.deletedAt),
        eq(submissions.state, "approved"),
      ),
    )
    .orderBy(desc(votes.createdAt));
  return rows.map(mapSubmission);
}

/* ── Tags ───────────────────────────────────────────────────────── */

export async function getAllTags(): Promise<Tag[]> {
  const rows = await db
    .select()
    .from(tags)
    .orderBy(tags.sortOrder);
  return rows.map((t) => ({
    slug: t.slug,
    name: t.name,
    tagline: t.tagline ?? "",
  }));
}

export async function getTagBySlug(slug: string): Promise<Tag | undefined> {
  const [t] = await db.select().from(tags).where(eq(tags.slug, slug)).limit(1);
  return t ? { slug: t.slug, name: t.name, tagline: t.tagline ?? "" } : undefined;
}

export async function getTopTags(): Promise<Array<Tag & { count: number }>> {
  const cutoff = new Date(Date.now() - 7 * 86_400_000);
  const rows = await db
    .select({
      slug: tags.slug,
      name: tags.name,
      tagline: tags.tagline,
      count: sql<number>`COUNT(${submissionTags.submissionId})::int`,
    })
    .from(tags)
    .leftJoin(submissionTags, eq(submissionTags.tagSlug, tags.slug))
    .leftJoin(
      submissions,
      and(
        eq(submissions.id, submissionTags.submissionId),
        eq(submissions.state, "approved"),
        isNull(submissions.deletedAt),
        gte(submissions.createdAt, cutoff),
      ),
    )
    .groupBy(tags.slug, tags.name, tags.tagline, tags.sortOrder)
    .orderBy(desc(sql`COUNT(${submissionTags.submissionId})`));
  return rows.map((r) => ({
    slug: r.slug,
    name: r.name,
    tagline: r.tagline ?? "",
    count: Number(r.count ?? 0),
  }));
}

export async function getSubmissionsByTag(
  slug: string,
  viewerId: string | null = null,
  limit?: number,
): Promise<Submission[]> {
  const muted = await getMutedTagSlugs(viewerId);
  const cap = clampLimit(limit);
  const q = db
    .select(SUBMISSION_BASE_SELECT)
    .from(submissions)
    .innerJoin(users, eq(users.id, submissions.authorId))
    .innerJoin(submissionTags, eq(submissionTags.submissionId, submissions.id))
    .where(
      and(
        eq(submissionTags.tagSlug, slug),
        FEED_BASE_FILTERS(),
        notInMutedTags(muted),
      ),
    )
    .orderBy(desc(HOT_RANK_EXPR));
  const rows = cap ? await q.limit(cap) : await q;
  return rows.map(mapSubmission);
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
        eq(submissions.state, "approved"),
        isNull(submissions.deletedAt),
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
