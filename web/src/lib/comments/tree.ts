/**
 * Pure tree-building for comment threads.
 *
 * Lives outside src/db/queries.ts so the test (and any other consumer
 * that doesn't need a DB client) can import it without triggering the
 * Neon initialization in src/db/client.ts.
 */

import type { CommentNode } from "@/lib/prototype-fixtures";

export type CommentRow = {
  id: string;
  parentId: string | null;
  body: string;
  state: "pending" | "approved" | "rejected";
  score: number;
  createdAt: Date;
  updatedAt: Date | null;
  authorUsername: string;
  authorImageUrl: string | null;
  authorIsAgent: boolean;
  deletedAt: Date | null;
};

function synthesizeVotes(score: number): { upvotes: number; downvotes: number } {
  return score >= 0
    ? { upvotes: score, downvotes: 0 }
    : { upvotes: 0, downvotes: -score };
}

/**
 * Build a CommentNode tree from a flat row list.
 *
 * Audit finding 3.1 — preserve thread structure when a parent is
 * filtered (rejected) but has approved descendants. Build the full
 * tree first, then prune tombstone leaves (filtered/deleted nodes
 * with no visible children).
 *
 * `publicOnly = true` filters out non-approved leaves (rejected,
 * pending). `false` keeps everything for staff/author views.
 */
export function buildCommentTree(
  rows: CommentRow[],
  publicOnly: boolean,
): CommentNode[] {
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
        if (tombstoned && children.length === 0) return null;
        const { upvotes, downvotes } = synthesizeVotes(r.score);
        return {
          id: r.id,
          user: tombstoned ? "[deleted]" : r.authorUsername,
          // Plumb avatar through. The previous shape dropped this
          // field, which forced every comment byline to fall through
          // to the boring-avatars identicon (Tier 3 in
          // components/prototype/Avatar.tsx) — including bots whose
          // users.image was set. Same for is_agent below: without
          // it, the byline can't render the AI chip.
          user_image_url: tombstoned ? null : r.authorImageUrl,
          user_is_agent: tombstoned ? false : r.authorIsAgent,
          submitted_at: r.createdAt.toISOString(),
          updated_at: tombstoned ? undefined : r.updatedAt?.toISOString(),
          upvotes: tombstoned ? 0 : upvotes,
          downvotes: tombstoned ? 0 : downvotes,
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
