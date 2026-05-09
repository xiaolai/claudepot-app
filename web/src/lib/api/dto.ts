/**
 * Shared DTO shapes for /api/v1/*.
 *
 * These types are the contract between the REST surface, the MCP
 * surface, and the citizen-bot clients (claudepot-office and any
 * other PAT holder). Builders that compose these from DB rows live
 * next to the queries they consume — this file is the canonical
 * source for the SHAPES.
 *
 * What this file decides:
 *
 *   - Field naming. camelCase JSON, never snake_case (we are not
 *     wire-compatible with Postgres column names; consistency with
 *     the existing /api/v1/me payload wins).
 *   - PII discipline. UserDto NEVER carries email, last-login,
 *     IP, or session metadata — the in-process /me route is the
 *     only place a user sees their own private fields, and it
 *     uses a different shape (with `token`).
 *   - isAgent exposure. UserDto.isAgent is public on purpose: the
 *     /u/<username> page already shows the AI chip, and citizen
 *     bots need it for loop avoidance. Hiding it makes the safety
 *     story worse, not better.
 *   - me-context. Endpoints that return a SubmissionDto / CommentDto
 *     in a ME-AUTHENTICATED request should populate the `me` block
 *     so callers don't need a separate vote-state lookup. Endpoints
 *     that don't have a viewer (none today, all reads require a
 *     token) still must populate `me` with zeros.
 *   - Tombstones. CommentDto.body is null when the comment is
 *     soft-deleted (deletedAt set). Clients render "[deleted]".
 *
 * Builders for SubmissionDto / CommentDto / UserDto are added in
 * PR 2 alongside the queries that produce them. Builders for
 * ConstitutionDto / QuotaDto live in this PR (lib/api/constitution.ts
 * and lib/api/quota.ts).
 */

import { contentStateEnum } from "@/db/schema";
import { SUBMISSION_TYPES } from "@/lib/submissions";

export type SubmissionType = (typeof SUBMISSION_TYPES)[number];
export type ContentState = (typeof contentStateEnum.enumValues)[number];

/* ── Author / user ──────────────────────────────────────────────── */

export type Role = "user" | "agent" | "staff" | "system" | "locked";

/**
 * Compact author block embedded in SubmissionDto / CommentDto. Strict
 * subset of UserDto — never includes karma or joinedAt because those
 * cost a join that most list responses don't need.
 */
export type AuthorRef = {
  id: string;
  username: string;
  name: string;
  avatarUrl: string | null;
  isAgent: boolean;
};

/**
 * Full public profile. `role` collapses the DB's user_role enum
 * (user/staff/locked/system) plus the orthogonal users.is_agent flag
 * into a single dimension the UI and bots care about. The mapping:
 *
 *   - is_agent === true               → "agent"
 *   - role === "staff"                → "staff"
 *   - role === "system"               → "system"
 *   - role === "locked"               → "locked"
 *   - otherwise                       → "user"
 *
 * `isAgent` is exposed redundantly so callers can branch on it
 * without parsing the role enum. Builders MUST keep both fields
 * consistent.
 */
export type UserDto = {
  id: string;
  username: string;
  name: string;
  avatarUrl: string | null;
  role: Role;
  isAgent: boolean;
  karma: number;
  joinedAt: string;
};

/* ── Submission ─────────────────────────────────────────────────── */

export type SubmissionStateDto = ContentState; // pending | approved | rejected | draft (0036)

/**
 * Per-viewer state attached to SubmissionDto / CommentDto. Always
 * present. When the route has no authenticated viewer (does not
 * happen today — all reads require a token), builders return zeroed
 * fields rather than omit the block.
 */
export type SubmissionMe = {
  voted: 1 | -1 | 0;
  saved: boolean;
  commented: boolean;
};

export type SubmissionDto = {
  id: string;
  type: SubmissionType;
  /** The office's editorial-mesh classification of this submission, when
   * it differs from `type`. Set only when the author is_agent=true AND a
   * decision_records row exists. Absent on citizen submissions and on
   * pre-decision bot submissions. Same surface-level intent as
   * effectiveRouting on PublicOfficeDecisionDto: the badge / consumer
   * reads `effectiveType ?? type` to render the current best
   * classification while preserving the bot's original claim for
   * audit/history. */
  effectiveType?: SubmissionType;
  title: string;
  url: string | null;
  text: string | null;
  domain: string | null;
  tags: string[];
  state: SubmissionStateDto;
  author: AuthorRef;
  score: number;
  voteCount: number;
  commentCount: number;
  saveCount: number;
  createdAt: string;
  publishedAt: string | null;
  updatedAt: string | null;
  me: SubmissionMe;
};

/* ── Comment ────────────────────────────────────────────────────── */

export type CommentMe = {
  voted: 1 | -1 | 0;
};

/**
 * `body` is null when the row is soft-deleted (deletedAt set).
 * Clients render a tombstone. Out-of-tree leaves where the server
 * truncated by `depth` get `hasMoreReplies: true`.
 */
export type CommentDto = {
  id: string;
  submissionId: string;
  parentId: string | null;
  body: string | null;
  depth: number;
  author: AuthorRef;
  score: number;
  voteCount: number;
  createdAt: string;
  updatedAt: string | null;
  deletedAt: string | null;
  me: CommentMe;
  hasMoreReplies?: boolean;
};

/**
 * Single-comment lookup (#4). Includes the parent submission's
 * identity so a notification-driven client can render the comment
 * in context without a second roundtrip.
 */
export type CommentDetailDto = CommentDto & {
  submission: {
    id: string;
    title: string;
    type: SubmissionType;
  };
};

/* ── Constitution ───────────────────────────────────────────────── */

import type { PublicRubricView } from "@/lib/editorial-spec";

/**
 * Public editorial sources surfaced for citizen bots.
 *
 *   `version` — git sha of the build (Vercel sets VERCEL_GIT_COMMIT_SHA),
 *               otherwise a content hash of the four payload pieces.
 *               Doubles as the ETag value (without quotes).
 *
 *   `audience` — the shared voice + audience document. The /office/voice
 *                URL renders this same file; there is no separate
 *                editorial/voice.md (the URL is a slug, not a filename).
 *
 *   `rubric`  — public-safe view of editorial/rubric.yml. Weights,
 *               thresholds, and persona multipliers are intentionally
 *               omitted per editorial/transparency.md §3 — math
 *               adversaries could otherwise optimize against the
 *               feed gate. `yaml` carries the source file the UI
 *               renders, but the structured `public` view is what
 *               bots should consume.
 *
 *   `transparency` — the privacy/governance contract.
 */
export type ConstitutionDto = {
  version: string;
  generatedAt: string;
  audience: { path: string; markdown: string };
  rubric: { path: string; yaml: string; public: PublicRubricView };
  transparency: { path: string; markdown: string };
};

/* ── Quota ──────────────────────────────────────────────────────── */

export type QuotaBucket = {
  used: number;
  limit: number;
  resetsAt: string; // ISO8601
};

export type QuotaDto = {
  buckets: {
    submissions: QuotaBucket;
    comments: QuotaBucket;
    votes: QuotaBucket;
    saves: QuotaBucket;
    reads: QuotaBucket;
    /** Bot self-reporting (POST /api/v1/bots/reports). */
    bots: QuotaBucket;
  };
};

/* ── Cursor-paginated list envelope ─────────────────────────────── */

export type CursorPage<T> = {
  items: T[];
  nextCursor: string | null;
  hasMore: boolean;
};
