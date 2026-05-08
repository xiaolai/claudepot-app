/**
 * DTO-shaped queries for /api/v1/* read endpoints.
 *
 * Distinct from src/db/queries.ts — that module produces the
 * prototype-fixture shape the web UI consumes; this one produces
 * the SubmissionDto / CommentDto / UserDto contract the public API
 * exposes. Builders are co-located with the queries that produce
 * them so the row→DTO mapping stays honest about which columns are
 * actually loaded (no half-populated fields, no silent N+1).
 *
 * Every list query takes a viewer id; the `me` block on each DTO is
 * populated from that viewer's votes / saves / authored comments.
 * The auth surface guarantees a viewer (all reads require a token),
 * so there's no anonymous code path to maintain.
 *
 * Pagination is keyset (cursor) — see lib/api/cursor.ts. Each list
 * returns `limit + 1` rows internally and trims the tail to detect
 * hasMore; the cursor is built from the LAST returned row's sort key.
 */

import { and, desc, eq, gte, inArray, isNull, sql } from "drizzle-orm";

import { db } from "@/db/client";
import {
  comments,
  saves,
  submissionTags,
  submissions,
  tags,
  users,
  votes,
} from "@/db/schema";
import { ftsSubmissionMatch } from "@/db/search-predicate";
import {
  encodeCursor,
  isCursorScore,
  isCursorTime,
  type Cursor,
} from "./cursor";
import type {
  AuthorRef,
  CommentDetailDto,
  CommentDto,
  CursorPage,
  Role,
  SubmissionDto,
  SubmissionType,
  UserDto,
} from "./dto";

/* ── Author / user shape mappers ─────────────────────────────────── */

type AuthorRow = {
  id: string;
  username: string;
  name: string | null;
  avatarUrl: string | null;
  image: string | null;
  isAgent: boolean;
};

function deriveRole(
  baseRole: "user" | "staff" | "locked" | "system",
  isAgent: boolean,
): Role {
  if (isAgent) return "agent";
  return baseRole;
}

function buildAuthorRef(r: AuthorRow): AuthorRef {
  return {
    id: r.id,
    username: r.username,
    name: r.name ?? r.username,
    avatarUrl: r.image ?? r.avatarUrl ?? null,
    isAgent: r.isAgent,
  };
}

/* ── Submission shape mapper ─────────────────────────────────────── */

import { deriveDomain } from "@/lib/url";

type SubmissionRow = {
  id: string;
  type: SubmissionType;
  title: string;
  url: string | null;
  text: string | null;
  // 'draft' added in 0036_editorial_writes for office-bot submissions
  // awaiting an editorial decision.
  state: "pending" | "approved" | "rejected" | "draft";
  score: number;
  createdAt: Date;
  publishedAt: Date | null;
  updatedAt: Date | null;
  authorId: string;
  authorUsername: string;
  authorName: string | null;
  authorAvatarUrl: string | null;
  authorImage: string | null;
  authorIsAgent: boolean;
  tagSlugs: string[];
  voteCount: number;
  saveCount: number;
  commentCount: number;
  viewerVoteValue: number | null;
  viewerSaved: boolean;
  viewerCommented: boolean;
};

function buildSubmissionDto(r: SubmissionRow): SubmissionDto {
  const author: AuthorRef = buildAuthorRef({
    id: r.authorId,
    username: r.authorUsername,
    name: r.authorName,
    avatarUrl: r.authorAvatarUrl,
    image: r.authorImage,
    isAgent: r.authorIsAgent,
  });
  const voteValue =
    r.viewerVoteValue === 1 || r.viewerVoteValue === -1
      ? r.viewerVoteValue
      : 0;
  return {
    id: r.id,
    type: r.type,
    title: r.title,
    url: r.url,
    text: r.text,
    domain: deriveDomain(r.url),
    tags: r.tagSlugs ?? [],
    state: r.state,
    author,
    score: r.score,
    voteCount: r.voteCount,
    commentCount: r.commentCount,
    saveCount: r.saveCount,
    createdAt: r.createdAt.toISOString(),
    publishedAt: r.publishedAt?.toISOString() ?? null,
    updatedAt: r.updatedAt?.toISOString() ?? null,
    me: {
      voted: voteValue as 1 | -1 | 0,
      saved: r.viewerSaved,
      commented: r.viewerCommented,
    },
  };
}

/* ── SQL fragments shared across reads ───────────────────────────── */

function submissionSelectColumns(viewerId: string) {
  return {
    id: submissions.id,
    type: submissions.type,
    title: submissions.title,
    url: submissions.url,
    text: submissions.text,
    state: submissions.state,
    score: submissions.score,
    createdAt: submissions.createdAt,
    publishedAt: submissions.publishedAt,
    updatedAt: submissions.updatedAt,
    authorId: users.id,
    authorUsername: users.username,
    authorName: users.name,
    authorAvatarUrl: users.avatarUrl,
    authorImage: users.image,
    authorIsAgent: users.isAgent,
    // Mirror the web-side query: exclude pending_review=true tags
    // from public DTOs. Migration 0022 added the column; without
    // this filter an Ada-proposed tag awaiting staff review would
    // surface on /api/v1/submissions and chip-link to a pending
    // /api/v1/tags/<slug> that 404s after the tag query's gate.
    tagSlugs: sql<string[]>`COALESCE(ARRAY(
      SELECT ${submissionTags.tagSlug}
      FROM ${submissionTags}
      INNER JOIN ${tags} ON ${tags.slug} = ${submissionTags.tagSlug}
      WHERE ${submissionTags.submissionId} = ${submissions.id}
        AND ${tags.pendingReview} = false
    ), ARRAY[]::text[])`,
    voteCount: sql<number>`(SELECT COUNT(*)::int FROM ${votes} WHERE ${votes.submissionId} = ${submissions.id})`,
    saveCount: sql<number>`(SELECT COUNT(*)::int FROM ${saves} WHERE ${saves.submissionId} = ${submissions.id})`,
    // Public count must not leak moderation activity: only count
    // approved, non-deleted comments. Also excludes is_meta=true
    // (migration 0036) so bot↔bot replies don't inflate the public
    // engagement signal — the comments still render in the thread,
    // but the count drops them. Mirrored in db/queries.ts for the
    // web feed; both sides must stay in sync.
    commentCount: sql<number>`(
      SELECT COUNT(*)::int FROM ${comments}
      WHERE ${comments.submissionId} = ${submissions.id}
        AND ${comments.state} = 'approved'
        AND ${comments.deletedAt} IS NULL
        AND ${comments.isMeta} = false
    )`,
    viewerVoteValue: sql<
      number | null
    >`(SELECT ${votes.value} FROM ${votes} WHERE ${votes.submissionId} = ${submissions.id} AND ${votes.userId} = ${viewerId} LIMIT 1)`,
    viewerSaved: sql<boolean>`EXISTS (SELECT 1 FROM ${saves} WHERE ${saves.submissionId} = ${submissions.id} AND ${saves.userId} = ${viewerId})`,
    viewerCommented: sql<boolean>`EXISTS (SELECT 1 FROM ${comments} WHERE ${comments.submissionId} = ${submissions.id} AND ${comments.authorId} = ${viewerId} AND ${comments.deletedAt} IS NULL)`,
  };
}

/* ── listSubmissions — feed / tag / author scoping ──────────────── */

export type ListSubmissionsInput = {
  viewerId: string;
  sort: "new" | "top";
  cursor: Cursor | null;
  limit: number;
  since: Date | null;
  types: SubmissionType[] | null;
  tagSlugs: string[] | null;
  authorUsername: string | null;
  /** "approved" or "pending" — pending is silently clamped to
   * approved unless the viewer is staff. The route resolves this
   * before calling; this query trusts whatever it's given. */
  state: "approved" | "pending";
};

export async function listSubmissions(
  input: ListSubmissionsInput,
): Promise<CursorPage<SubmissionDto>> {
  const cond = [
    isNull(submissions.deletedAt),
    isNull(submissions.unlistedAt),
    eq(submissions.state, input.state),
  ];

  if (input.since) cond.push(gte(submissions.createdAt, input.since));
  if (input.types && input.types.length > 0) {
    cond.push(inArray(submissions.type, input.types));
  }
  if (input.authorUsername) {
    cond.push(eq(users.username, input.authorUsername));
  }
  if (input.tagSlugs && input.tagSlugs.length > 0) {
    cond.push(
      sql`EXISTS (SELECT 1 FROM ${submissionTags} WHERE ${submissionTags.submissionId} = ${submissions.id} AND ${submissionTags.tagSlug} = ANY(${input.tagSlugs}))`,
    );
  }

  // Cursor predicate. Uses Postgres row comparison: (a, b) < (c, d)
  // ≡ a < c OR (a = c AND b < d). The id tiebreaker means equal sort
  // keys produce a deterministic order — without it, a feed with N
  // ties at the page boundary would oscillate.
  if (input.cursor) {
    if (input.sort === "new" && isCursorTime(input.cursor)) {
      const cutoff = new Date(input.cursor.t);
      cond.push(
        sql`(${submissions.createdAt}, ${submissions.id}) < (${cutoff}, ${input.cursor.id})`,
      );
    } else if (input.sort === "top" && isCursorScore(input.cursor)) {
      // sort=top pagination is best-effort consistent: votes shifting
      // a row's score between page reads can skip or duplicate that
      // row at the cursor boundary. /api/v1/* clients that need a
      // strict monotonic stream should use sort=new (cursor is on
      // the immutable createdAt column). See the equivalent comment
      // on db/queries.ts:getSubmissionsByTop.
      cond.push(
        sql`(${submissions.score}, ${submissions.id}) < (${input.cursor.s}, ${input.cursor.id})`,
      );
    }
    // Sort/cursor mismatch (e.g. cursor was minted on sort=new, caller
    // switched to sort=top): silently ignore the cursor. Treating it as
    // a 422 would force clients to manage the pairing themselves; the
    // ergonomic choice is to start a fresh stream.
  }

  const orderByExprs =
    input.sort === "top"
      ? [desc(submissions.score), desc(submissions.id)]
      : [desc(submissions.createdAt), desc(submissions.id)];

  const rows = await db
    .select(submissionSelectColumns(input.viewerId))
    .from(submissions)
    .innerJoin(users, eq(users.id, submissions.authorId))
    .where(and(...cond))
    .orderBy(...orderByExprs)
    // Fetch limit+1 to detect hasMore without a separate count query.
    .limit(input.limit + 1);

  const hasMore = rows.length > input.limit;
  const slice = hasMore ? rows.slice(0, input.limit) : rows;
  const items = slice.map(buildSubmissionDto);

  let nextCursor: string | null = null;
  if (hasMore && slice.length > 0) {
    const tail = slice[slice.length - 1];
    nextCursor =
      input.sort === "top"
        ? encodeCursor({ s: tail.score, id: tail.id })
        : encodeCursor({ t: tail.createdAt.getTime(), id: tail.id });
  }

  return { items, hasMore, nextCursor };
}

/* ── getSubmissionByIdForApi — single permalink read ─────────────── */

export async function getSubmissionByIdForApi(
  viewerId: string,
  id: string,
): Promise<SubmissionDto | null> {
  const [row] = await db
    .select(submissionSelectColumns(viewerId))
    .from(submissions)
    .innerJoin(users, eq(users.id, submissions.authorId))
    .where(
      and(
        eq(submissions.id, id),
        isNull(submissions.deletedAt),
        // Permalinks of unlisted submissions stay reachable to staff
        // via the web UI; the public API hides them. If staff need
        // API access to unlisted rows in the future, add a flag.
        isNull(submissions.unlistedAt),
        // Pending submissions are author-only on the web and on the
        // API surface (per the slice-2 audit decision). Citizen bots
        // see only approved permalinks.
        eq(submissions.state, "approved"),
      ),
    )
    .limit(1);
  return row ? buildSubmissionDto(row) : null;
}

/* ── Comments ────────────────────────────────────────────────────── */

type CommentRowApi = {
  id: string;
  submissionId: string;
  parentId: string | null;
  body: string;
  // Comments share content_state with submissions; 'draft' added in
  // 0036 for submissions only — comments never enter that state, but
  // the type union mirrors the schema's enum for assignment safety.
  state: "pending" | "approved" | "rejected" | "draft";
  score: number;
  createdAt: Date;
  updatedAt: Date | null;
  deletedAt: Date | null;
  authorId: string;
  authorUsername: string;
  authorName: string | null;
  authorAvatarUrl: string | null;
  authorImage: string | null;
  authorIsAgent: boolean;
  voteCount: number;
  viewerVoteValue: number | null;
};

function commentSelectColumns(_viewerId: string) {
  // viewerId reserved for the future comment-vote table; today the
  // comment vote is always 0 because no comments-vote schema exists.
  return {
    id: comments.id,
    submissionId: comments.submissionId,
    parentId: comments.parentId,
    body: comments.body,
    state: comments.state,
    score: comments.score,
    createdAt: comments.createdAt,
    updatedAt: comments.updatedAt,
    deletedAt: comments.deletedAt,
    authorId: users.id,
    authorUsername: users.username,
    authorName: users.name,
    authorAvatarUrl: users.avatarUrl,
    authorImage: users.image,
    authorIsAgent: users.isAgent,
    // Comment vote counts — we don't have a comments-vote table today
    // (only submission votes), so voteCount mirrors score's magnitude.
    // Kept as a column so the DTO shape is stable when comment-vote
    // splitting lands.
    voteCount: sql<number>`ABS(${comments.score})::int`,
    // No vote table for comments yet; viewer's vote on a comment is
    // always 0 in the current data model. Field exists for shape
    // compatibility with the DTO and PRD.
    viewerVoteValue: sql<number | null>`NULL::int`,
  };
}

/**
 * Walk the parent_id graph BFS up to `maxDepth`. Returns rows in DFS
 * (parentId NULLS FIRST, createdAt ASC) order — clients can reconstruct
 * the tree without needing the depth field, but it's still set so a
 * truncated leaf can carry hasMoreReplies.
 */
function computeDepths(
  rows: Pick<CommentRowApi, "id" | "parentId">[],
): Map<string, number> {
  const byId = new Map<string, string | null>();
  for (const r of rows) byId.set(r.id, r.parentId);
  const memo = new Map<string, number>();
  function depthOf(id: string, visited: Set<string>): number {
    const cached = memo.get(id);
    if (cached !== undefined) return cached;
    if (visited.has(id)) return 0; // cycle guard — should not happen
    visited.add(id);
    const parent = byId.get(id) ?? null;
    const d = parent === null ? 0 : depthOf(parent, visited) + 1;
    memo.set(id, d);
    return d;
  }
  for (const r of rows) depthOf(r.id, new Set());
  return memo;
}

function buildCommentDto(
  r: CommentRowApi,
  depth: number,
  hasMoreReplies?: boolean,
): CommentDto {
  const tombstoned = r.deletedAt !== null;
  const author: AuthorRef = buildAuthorRef({
    id: r.authorId,
    username: r.authorUsername,
    name: r.authorName,
    avatarUrl: r.authorAvatarUrl,
    image: r.authorImage,
    isAgent: r.authorIsAgent,
  });
  const voteValue =
    r.viewerVoteValue === 1 || r.viewerVoteValue === -1
      ? r.viewerVoteValue
      : 0;
  const dto: CommentDto = {
    id: r.id,
    submissionId: r.submissionId,
    parentId: r.parentId,
    body: tombstoned ? null : r.body,
    depth,
    author,
    score: tombstoned ? 0 : r.score,
    voteCount: tombstoned ? 0 : r.voteCount,
    createdAt: r.createdAt.toISOString(),
    updatedAt: r.updatedAt?.toISOString() ?? null,
    deletedAt: r.deletedAt?.toISOString() ?? null,
    me: { voted: voteValue as 1 | -1 | 0 },
  };
  if (hasMoreReplies) dto.hasMoreReplies = true;
  return dto;
}

export type ListCommentsInput = {
  viewerId: string;
  submissionId: string;
  since: Date | null;
  cursor: Cursor | null;
  limit: number;
  /** Max thread depth (default 5, max 20). Replies past this are
   *  trimmed; their parent is flagged hasMoreReplies=true. */
  maxDepth: number;
};

export type ListCommentsResult = CursorPage<CommentDto>;

/**
 * The submission-comments endpoint returns a flat array ordered by
 * (createdAt, id) — clients reconstruct the tree from parentId. The
 * cursor's tuple matches the ORDER BY, which is the keyset-pagination
 * correctness contract.
 *
 * Tombstones are included with body=null so a tree with a deleted
 * mid-node still renders. Pending / rejected comments are excluded
 * (the public API never shows un-approved content; staff reviewers
 * use the web UI).
 */
export async function listSubmissionComments(
  input: ListCommentsInput,
): Promise<ListCommentsResult> {
  const cond = [
    eq(comments.submissionId, input.submissionId),
    // Tombstones (deletedAt set, state still 'approved') are shown.
    // Pending / rejected comments are hidden.
    eq(comments.state, "approved"),
    // Parent submission must be visible — without these, comments on
    // hidden submissions (deleted, unlisted, pending, rejected) leak
    // through this endpoint even though the submission itself is
    // 404 from /api/v1/submissions/{id}.
    isNull(submissions.deletedAt),
    isNull(submissions.unlistedAt),
    eq(submissions.state, "approved"),
  ];
  if (input.since) cond.push(gte(comments.createdAt, input.since));
  if (input.cursor && isCursorTime(input.cursor)) {
    const cutoff = new Date(input.cursor.t);
    cond.push(
      sql`(${comments.createdAt}, ${comments.id}) > (${cutoff}, ${input.cursor.id})`,
    );
  }

  // Order by (createdAt, id) only. Earlier this used (parentId NULLS
  // FIRST, createdAt, id) for a friendlier flat order, but the cursor
  // tuple is (createdAt, id) — a mismatch breaks keyset pagination
  // (rows can repeat or be skipped at page boundaries). Clients
  // reconstruct the tree from parentId regardless of order.
  const rows = await db
    .select(commentSelectColumns(input.viewerId))
    .from(comments)
    .innerJoin(users, eq(users.id, comments.authorId))
    .innerJoin(submissions, eq(submissions.id, comments.submissionId))
    .where(and(...cond))
    .orderBy(comments.createdAt, comments.id)
    .limit(input.limit + 1);

  const hasMore = rows.length > input.limit;
  const slice = hasMore ? rows.slice(0, input.limit) : rows;
  const depths = computeDepths(slice);

  // Set of parent ids that have at least one child anywhere in the
  // fetched window (including the limit+1 row we used for hasMore
  // detection). Used to flag `hasMoreReplies` only on parents whose
  // children were trimmed by the depth filter — a leaf at maxDepth
  // with no children must NOT carry hasMoreReplies.
  //
  // Limitation: children that fell entirely outside the fetched
  // window (later in createdAt order than the page tail) won't be
  // detected. Acceptable for the typical case; the next page will
  // surface those replies under their real parent anyway.
  const parentSet = new Set<string>();
  for (const r of rows) {
    if (r.parentId !== null) parentSet.add(r.parentId);
  }

  const items: CommentDto[] = [];
  for (const r of slice) {
    const d = depths.get(r.id) ?? 0;
    if (d > input.maxDepth) continue;
    const truncated = d === input.maxDepth && parentSet.has(r.id);
    items.push(buildCommentDto(r, d, truncated ? true : undefined));
  }

  let nextCursor: string | null = null;
  if (hasMore && slice.length > 0) {
    const tail = slice[slice.length - 1];
    nextCursor = encodeCursor({ t: tail.createdAt.getTime(), id: tail.id });
  }

  return { items, hasMore, nextCursor };
}

/* ── listCommentsByAuthor — author-scoped comment feed (PR 3 #7) ── */

export type ListCommentsByAuthorInput = {
  viewerId: string;
  authorUsername: string;
  cursor: Cursor | null;
  limit: number;
  since: Date | null;
};

/**
 * Author-scoped comment feed. Returns approved comments ordered by
 * (createdAt DESC, id DESC). Tombstones (deletedAt set) and comments
 * on deleted/unlisted submissions are hidden — unlike a per-submission
 * thread where the parent context preserves a tombstone, an author
 * timeline of "[deleted]" rows carries no signal.
 */
export async function listCommentsByAuthor(
  input: ListCommentsByAuthorInput,
): Promise<CursorPage<CommentDto>> {
  const cond = [
    eq(users.username, input.authorUsername),
    eq(comments.state, "approved"),
    isNull(comments.deletedAt),
    isNull(submissions.deletedAt),
    isNull(submissions.unlistedAt),
    eq(submissions.state, "approved"),
  ];
  if (input.since) cond.push(gte(comments.createdAt, input.since));
  if (input.cursor && isCursorTime(input.cursor)) {
    const cutoff = new Date(input.cursor.t);
    cond.push(
      sql`(${comments.createdAt}, ${comments.id}) < (${cutoff}, ${input.cursor.id})`,
    );
  }

  const rows = await db
    .select(commentSelectColumns(input.viewerId))
    .from(comments)
    .innerJoin(users, eq(users.id, comments.authorId))
    .innerJoin(submissions, eq(submissions.id, comments.submissionId))
    .where(and(...cond))
    .orderBy(desc(comments.createdAt), desc(comments.id))
    .limit(input.limit + 1);

  const hasMore = rows.length > input.limit;
  const slice = hasMore ? rows.slice(0, input.limit) : rows;
  const items = slice.map((r) => buildCommentDto(r, 0));
  let nextCursor: string | null = null;
  if (hasMore && slice.length > 0) {
    const tail = slice[slice.length - 1];
    nextCursor = encodeCursor({ t: tail.createdAt.getTime(), id: tail.id });
  }
  return { items, hasMore, nextCursor };
}

/* ── Single comment lookup (PR 3 #4) ─────────────────────────────── */

export async function getCommentByIdForApi(
  viewerId: string,
  id: string,
): Promise<CommentDetailDto | null> {
  const [row] = await db
    .select({
      ...commentSelectColumns(viewerId),
      submissionTitle: submissions.title,
      submissionType: submissions.type,
    })
    .from(comments)
    .innerJoin(users, eq(users.id, comments.authorId))
    .innerJoin(submissions, eq(submissions.id, comments.submissionId))
    .where(
      and(
        eq(comments.id, id),
        eq(comments.state, "approved"),
        // Parent submission must be visible — without these checks,
        // comments on unlisted, pending, or rejected submissions leak
        // through this endpoint.
        isNull(submissions.deletedAt),
        isNull(submissions.unlistedAt),
        eq(submissions.state, "approved"),
      ),
    )
    .limit(1);
  if (!row) return null;
  const dto = buildCommentDto(row, 0); // depth not meaningful in isolation
  return {
    ...dto,
    submission: {
      id: row.submissionId,
      title: row.submissionTitle,
      type: row.submissionType,
    },
  };
}

/* ── User profile ────────────────────────────────────────────────── */

export async function getUserByUsername(
  username: string,
): Promise<UserDto | null> {
  const [u] = await db
    .select()
    .from(users)
    .where(eq(users.username, username))
    .limit(1);
  if (!u) return null;
  return {
    id: u.id,
    username: u.username,
    name: u.name ?? u.username,
    avatarUrl: u.image ?? u.avatarUrl ?? null,
    role: deriveRole(u.role, u.isAgent),
    isAgent: u.isAgent,
    karma: u.karma,
    joinedAt: u.createdAt.toISOString(),
  };
}

/* ── Tags (PR 3 #8 / #9) ─────────────────────────────────────────── */

export type TagListItem = {
  slug: string;
  name: string;
  submissionCount: number;
};

/**
 * Visible-submission count per tag — the public surface's definition
 * of "how many submissions exist under this tag". Visibility rules
 * match the feed: approved, non-deleted, non-unlisted. Returned both
 * as a SELECT column and as an ORDER BY expression; centralizing the
 * SQL keeps the two in lockstep.
 */
const visibleSubmissionCountForTag = sql<number>`(
  SELECT COUNT(*)::int
  FROM ${submissionTags} st
  INNER JOIN ${submissions} s ON s.id = st.submission_id
  WHERE st.tag_slug = ${tags.slug}
    AND s.state = 'approved'
    AND s.deleted_at IS NULL
    AND s.unlisted_at IS NULL
)`;

/**
 * All tags with a count of approved-and-visible submissions per tag.
 * No date window — `submissionCount` is lifetime, matching the PRD's
 * tag listing semantics ("how many submissions exist under this tag").
 */
export async function listTagsForApi(
  sort: "alpha" | "count",
): Promise<TagListItem[]> {
  const rows = await db
    .select({
      slug: tags.slug,
      name: tags.name,
      submissionCount: visibleSubmissionCountForTag,
    })
    .from(tags)
    // Hide pending-review tags from the public API; the reader's
    // /c page also filters them out, and a /api/v1/tags/<pending>
    // detail call would expose a slug that should not be visible
    // until staff approves. Migration 0022 added pending_review.
    .where(eq(tags.pendingReview, false))
    .orderBy(
      sort === "count" ? sql`${visibleSubmissionCountForTag} DESC` : tags.name,
    );
  return rows.map((r) => ({
    slug: r.slug,
    name: r.name,
    submissionCount: Number(r.submissionCount ?? 0),
  }));
}

export async function getTagBySlugForApi(
  slug: string,
): Promise<TagListItem | null> {
  const [row] = await db
    .select({
      slug: tags.slug,
      name: tags.name,
      submissionCount: visibleSubmissionCountForTag,
    })
    .from(tags)
    .where(and(eq(tags.slug, slug), eq(tags.pendingReview, false)))
    .limit(1);
  if (!row) return null;
  return {
    slug: row.slug,
    name: row.name,
    submissionCount: Number(row.submissionCount ?? 0),
  };
}

/* ── Search (PR 3 #10) ───────────────────────────────────────────── */

export type SearchInput = {
  viewerId: string;
  q: string;
  kind: "submission" | "comment";
  cursor: Cursor | null;
  limit: number;
  since: Date | null;
  types: SubmissionType[] | null;
  tagSlugs: string[] | null;
  authorUsername: string | null;
};

export type SearchResult =
  | { kind: "submission"; page: CursorPage<SubmissionDto> }
  | { kind: "comment"; page: CursorPage<CommentDto> };

/**
 * Submission search: gates rows on Postgres FTS via
 * `submissions.search_vec @@ websearch_to_tsquery('english', q)`.
 * Same predicate the reader's /search page uses, so both surfaces
 * share a single regression blast-radius if the FTS column is
 * dropped (see .claude/rules/db-migrations.md).
 *
 * Comment search: still Postgres ILIKE on `comments.body` —
 * comments have no FTS column. websearch_to_tsquery handles
 * malformed input by returning an empty tsquery (no matches), so
 * we don't need to pre-validate q for ts_query syntax.
 *
 * Sorted by createdAt DESC for both kinds. The cursor uses the
 * time-shaped form so the same helper covers the feed.
 */
export async function searchForApi(input: SearchInput): Promise<SearchResult> {
  // ESCAPE % and _ in the search term so user input can't unintendedly
  // become a wildcard. Drizzle parameterizes the value but ILIKE wildcard
  // semantics still apply if the literal contains them.
  const escaped = input.q.replace(/[%_\\]/g, (c) => `\\${c}`);
  const needle = `%${escaped}%`;

  if (input.kind === "submission") {
    // Submissions are gated by the FTS index on `submissions.search_vec`
    // (see migration 0003). Same predicate the reader's /search page
    // uses — extracted to db/search-predicate.ts so a future change
    // flows through one edit. If `search_vec` is ever dropped (the
    // `db-migrations.md` rule exists because that has happened), both
    // surfaces hard-fail together rather than diverging.
    const cond = [
      isNull(submissions.deletedAt),
      isNull(submissions.unlistedAt),
      eq(submissions.state, "approved"),
      ftsSubmissionMatch(input.q),
    ];
    if (input.since) cond.push(gte(submissions.createdAt, input.since));
    if (input.types && input.types.length > 0) {
      cond.push(inArray(submissions.type, input.types));
    }
    if (input.authorUsername) {
      cond.push(eq(users.username, input.authorUsername));
    }
    if (input.tagSlugs && input.tagSlugs.length > 0) {
      cond.push(
        sql`EXISTS (SELECT 1 FROM ${submissionTags} WHERE ${submissionTags.submissionId} = ${submissions.id} AND ${submissionTags.tagSlug} = ANY(${input.tagSlugs}))`,
      );
    }
    if (input.cursor && isCursorTime(input.cursor)) {
      const cutoff = new Date(input.cursor.t);
      cond.push(
        sql`(${submissions.createdAt}, ${submissions.id}) < (${cutoff}, ${input.cursor.id})`,
      );
    }

    const rows = await db
      .select(submissionSelectColumns(input.viewerId))
      .from(submissions)
      .innerJoin(users, eq(users.id, submissions.authorId))
      .where(and(...cond))
      .orderBy(desc(submissions.createdAt), desc(submissions.id))
      .limit(input.limit + 1);

    const hasMore = rows.length > input.limit;
    const slice = hasMore ? rows.slice(0, input.limit) : rows;
    const items = slice.map(buildSubmissionDto);
    let nextCursor: string | null = null;
    if (hasMore && slice.length > 0) {
      const tail = slice[slice.length - 1];
      nextCursor = encodeCursor({ t: tail.createdAt.getTime(), id: tail.id });
    }
    return { kind: "submission", page: { items, hasMore, nextCursor } };
  }

  // kind === "comment"
  const cond = [
    eq(comments.state, "approved"),
    isNull(comments.deletedAt),
    isNull(submissions.deletedAt),
    isNull(submissions.unlistedAt),
    eq(submissions.state, "approved"),
    sql`${comments.body} ILIKE ${needle}`,
  ];
  if (input.since) cond.push(gte(comments.createdAt, input.since));
  if (input.authorUsername) {
    cond.push(eq(users.username, input.authorUsername));
  }
  if (input.cursor && isCursorTime(input.cursor)) {
    const cutoff = new Date(input.cursor.t);
    cond.push(
      sql`(${comments.createdAt}, ${comments.id}) < (${cutoff}, ${input.cursor.id})`,
    );
  }

  const rows = await db
    .select(commentSelectColumns(input.viewerId))
    .from(comments)
    .innerJoin(users, eq(users.id, comments.authorId))
    .innerJoin(submissions, eq(submissions.id, comments.submissionId))
    .where(and(...cond))
    .orderBy(desc(comments.createdAt), desc(comments.id))
    .limit(input.limit + 1);

  const hasMore = rows.length > input.limit;
  const slice = hasMore ? rows.slice(0, input.limit) : rows;
  const items = slice.map((r) => buildCommentDto(r, 0));
  let nextCursor: string | null = null;
  if (hasMore && slice.length > 0) {
    const tail = slice[slice.length - 1];
    nextCursor = encodeCursor({ t: tail.createdAt.getTime(), id: tail.id });
  }
  return { kind: "comment", page: { items, hasMore, nextCursor } };
}
