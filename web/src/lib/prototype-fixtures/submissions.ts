/**
 * Submission, comment, and user queries against the prototype fixtures.
 */

import { commentsByPost, hotScore, publicVisible, submissions, users } from "./data";
import { commentEffectiveState, effectiveState } from "@/lib/moderation-fixtures";
import type { CommentNode, Submission, User } from "./types";

/* ── Submissions ───────────────────────────────────────────── */

export function getAllSubmissions(): Submission[] {
  return submissions;
}

export { hotScore };

export function getSubmissionsByHot(): Submission[] {
  return [...publicVisible()].sort((a, b) => hotScore(b) - hotScore(a));
}

export function getSubmissionsByNew(): Submission[] {
  return [...publicVisible()].sort(
    (a, b) =>
      new Date(b.submitted_at).getTime() - new Date(a.submitted_at).getTime(),
  );
}

export function getSubmissionsByTop(
  range: "day" | "week" | "all" = "day",
): Submission[] {
  const now = Date.now();
  const cutoff =
    range === "day"
      ? now - 86_400_000
      : range === "week"
        ? now - 7 * 86_400_000
        : 0;
  return publicVisible()
    .filter((s) => new Date(s.submitted_at).getTime() >= cutoff)
    .sort((a, b) => b.upvotes - b.downvotes - (a.upvotes - a.downvotes));
}

export function getSubmissionById(id: string): Submission | undefined {
  return submissions.find((s) => s.id === id);
}

/* ── Comments ──────────────────────────────────────────────── */

/** Comments visible to public — approved only. */
export function getCommentsForSubmission(id: string): CommentNode[] {
  return filterCommentTree(
    commentsByPost[id] ?? [],
    (c) => commentEffectiveState(c) === "approved",
  );
}

/** All comments including pending/rejected — used by the author and staff views. */
export function getAllCommentsForSubmission(id: string): CommentNode[] {
  return commentsByPost[id] ?? [];
}

function filterCommentTree(
  nodes: CommentNode[],
  predicate: (c: CommentNode) => boolean,
): CommentNode[] {
  return nodes
    .filter(predicate)
    .map((n) => ({ ...n, children: filterCommentTree(n.children, predicate) }));
}

/* ── Users ─────────────────────────────────────────────────── */

export function getUser(username: string): User | undefined {
  return users.find((u) => u.username === username);
}

export function getAllUsers(): User[] {
  return users;
}

/** Submissions by a user — public-visible by default; pass `includeAll` for the author's own view. */
export function getSubmissionsByUser(
  username: string,
  includeAll = false,
): Submission[] {
  const pool = includeAll ? submissions : publicVisible();
  return pool
    .filter((s) => s.user === username)
    .sort(
      (a, b) =>
        new Date(b.submitted_at).getTime() -
        new Date(a.submitted_at).getTime(),
    );
}

/** Pending or rejected submissions for a user — author's own pending/queue view. */
export function getPendingForUser(username: string): Submission[] {
  return submissions
    .filter((s) => s.user === username && effectiveState(s) !== "approved")
    .sort(
      (a, b) =>
        new Date(b.submitted_at).getTime() -
        new Date(a.submitted_at).getTime(),
    );
}

/**
 * Saved (★ private bookmark) — distinct from upvotes (▲ public signal).
 * Stub: deterministic per-user list keyed off username.
 */
export function getSavedForUser(username: string): Submission[] {
  const seed = username.charCodeAt(0);
  const order = [...publicVisible()].sort((a, b) => b.upvotes - a.upvotes);
  return order.slice(seed % 4, (seed % 4) + 6);
}

/** Submissions a user has upvoted (separate from saved). Stub. */
export function getUpvotedByUser(username: string): Submission[] {
  const seed = username.charCodeAt(0) + 1;
  return [...publicVisible()]
    .sort((a, b) => hotScore(b) - hotScore(a))
    .slice(seed % 5, (seed % 5) + 8);
}
