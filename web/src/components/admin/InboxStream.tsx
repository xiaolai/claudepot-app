import { and, count, desc, eq, inArray, isNull, like, not } from "drizzle-orm";

import { db } from "@/db/client";
import {
  flags,
  policyDecisions,
  submissions,
  users,
} from "@/db/schema";

import {
  InboxRow,
  type InboxItem,
  type AppealItem,
  type OpenFlagItem,
  type PendingSubmissionItem,
} from "./InboxRow";
import type { InboxFilter } from "./HeaderChips";

interface Props {
  asSuffix: string;
  filter: InboxFilter;
}

const STREAM_LIMIT = 50;

/**
 * Today inbox stream — time-ordered union of pending submissions,
 * open community flags (non-appeal), and open appeals.
 *
 * Pending tags don't appear here; they go in NoticeStrip because
 * they're vocabulary governance, not content moderation.
 *
 * Each kind is queried independently then merged in JS by
 * createdAt DESC. At STREAM_LIMIT=50 with three sources, this is
 * three small queries against indexed columns — cheaper than a
 * UNION over polymorphic types and easier to reason about.
 */
export async function InboxStream({ asSuffix, filter }: Props) {
  const [pendingSubs, openFlags, openAppeals] = await Promise.all([
    filter === "all" || filter === "submission"
      ? loadPendingSubmissions()
      : Promise.resolve([] as PendingSubmissionItem[]),
    filter === "all" || filter === "flag"
      ? loadOpenFlags()
      : Promise.resolve([] as OpenFlagItem[]),
    filter === "all" || filter === "appeal"
      ? loadOpenAppeals()
      : Promise.resolve([] as AppealItem[]),
  ]);

  const merged: InboxItem[] = [...pendingSubs, ...openFlags, ...openAppeals]
    .sort((a, b) => b.createdAt.getTime() - a.createdAt.getTime())
    .slice(0, STREAM_LIMIT);

  if (merged.length === 0) {
    return <EmptyState filter={filter} asSuffix={asSuffix} />;
  }

  return (
    <div className="proto-inbox-stream" data-inbox-stream="">
      {merged.map((item) => (
        <InboxRow key={`${item.kind}:${item.id}`} item={item} />
      ))}
    </div>
  );
}

function EmptyState({
  filter,
  asSuffix,
}: {
  filter: InboxFilter;
  asSuffix: string;
}) {
  if (filter !== "all") {
    return (
      <p className="proto-empty proto-empty-spaced">
        No items match this filter. <a href={`/admin${asSuffix}`}>Clear filter</a>.
      </p>
    );
  }
  return (
    <p className="proto-empty proto-empty-spaced">
      No work. The triage queue is clear.
    </p>
  );
}

async function loadPendingSubmissions(): Promise<PendingSubmissionItem[]> {
  const rows = await db
    .select({
      id: submissions.id,
      createdAt: submissions.createdAt,
      title: submissions.title,
      url: submissions.url,
      type: submissions.type,
      authorUsername: users.username,
      authorKarma: users.karma,
      authorJoinedAt: users.createdAt,
      authorId: users.id,
    })
    .from(submissions)
    .innerJoin(users, eq(users.id, submissions.authorId))
    .where(and(eq(submissions.state, "pending"), isNull(submissions.deletedAt)))
    .orderBy(desc(submissions.createdAt))
    .limit(STREAM_LIMIT);

  if (rows.length === 0) return [];

  // One round-trip to count prior posts per author. SELECT
  // author_id, COUNT(*) WHERE author_id IN (...) AND deleted_at IS
  // NULL — cheaper than N subqueries even at small N.
  const authorIds = rows.map((r) => r.authorId);
  const priorCounts = await db
    .select({
      authorId: submissions.authorId,
      n: count(),
    })
    .from(submissions)
    .where(
      and(
        inArray(submissions.authorId, authorIds),
        isNull(submissions.deletedAt),
      ),
    )
    .groupBy(submissions.authorId);
  const priorByAuthor = new Map<string, number>();
  for (const r of priorCounts) priorByAuthor.set(r.authorId, r.n);

  return rows.map((r) => ({
    kind: "pending_submission" as const,
    id: r.id,
    createdAt: r.createdAt,
    title: r.title,
    url: r.url,
    type: r.type,
    authorUsername: r.authorUsername,
    authorKarma: r.authorKarma,
    authorJoinedAt: r.authorJoinedAt,
    // Subtract 1 because the pending submission itself is in the
    // count if it has been inserted by now (it is — that's how we
    // got here). Floor at 0.
    authorPriorPosts: Math.max(0, (priorByAuthor.get(r.authorId) ?? 0) - 1),
  }));
}

async function loadOpenFlags(): Promise<OpenFlagItem[]> {
  // Non-appeal flags. We use NOT LIKE 'appeal:%' to exclude appeals
  // that share the flags table.
  const rows = await db
    .select({
      id: flags.id,
      createdAt: flags.createdAt,
      reason: flags.reason,
      targetType: flags.targetType,
      targetId: flags.targetId,
      reporterUsername: users.username,
    })
    .from(flags)
    .innerJoin(users, eq(users.id, flags.reporterId))
    .where(and(eq(flags.status, "open"), not(like(flags.reason, "appeal:%"))))
    .orderBy(desc(flags.createdAt))
    .limit(STREAM_LIMIT);

  if (rows.length === 0) return [];

  // Resolve target labels in batches by type — submissions get
  // titles, users get usernames. Comments today have no title; we
  // show the truncated id.
  const subTargetIds = rows
    .filter((r) => r.targetType === "submission")
    .map((r) => r.targetId);
  const userTargetIds = rows
    .filter((r) => r.targetType === "user")
    .map((r) => r.targetId);
  const [subRows, userRows] = await Promise.all([
    subTargetIds.length > 0
      ? db
          .select({ id: submissions.id, title: submissions.title })
          .from(submissions)
          .where(inArray(submissions.id, subTargetIds))
      : Promise.resolve([] as Array<{ id: string; title: string }>),
    userTargetIds.length > 0
      ? db
          .select({ id: users.id, username: users.username })
          .from(users)
          .where(inArray(users.id, userTargetIds))
      : Promise.resolve([] as Array<{ id: string; username: string }>),
  ]);
  const subTitleById = new Map(subRows.map((r) => [r.id, r.title]));
  const userUsernameById = new Map(userRows.map((r) => [r.id, r.username]));

  return rows.map((r) => ({
    kind: "open_flag" as const,
    id: r.id,
    createdAt: r.createdAt,
    reporter: r.reporterUsername,
    reason: r.reason,
    targetType: r.targetType,
    targetId: r.targetId,
    targetTitle: subTitleById.get(r.targetId) ?? null,
    targetUserUsername: userUsernameById.get(r.targetId) ?? null,
  }));
}

async function loadOpenAppeals(): Promise<AppealItem[]> {
  const rows = await db
    .select({
      id: flags.id,
      createdAt: flags.createdAt,
      reason: flags.reason,
      targetType: flags.targetType,
      targetId: flags.targetId,
      reporterUsername: users.username,
    })
    .from(flags)
    .innerJoin(users, eq(users.id, flags.reporterId))
    .where(and(eq(flags.status, "open"), like(flags.reason, "appeal:%")))
    .orderBy(desc(flags.createdAt))
    .limit(STREAM_LIMIT);

  if (rows.length === 0) return [];

  // Latest decision per (targetType, targetId) — same pattern as
  // /admin/log.
  const targetIds = rows.map((r) => r.targetId);
  const decisions = await db
    .select({
      targetType: policyDecisions.targetType,
      targetId: policyDecisions.targetId,
      category: policyDecisions.category,
      confidence: policyDecisions.confidence,
      oneLineWhy: policyDecisions.oneLineWhy,
      decidedAt: policyDecisions.decidedAt,
    })
    .from(policyDecisions)
    .where(inArray(policyDecisions.targetId, targetIds))
    .orderBy(desc(policyDecisions.decidedAt));
  const decisionByTarget = new Map<
    string,
    {
      category: string | null;
      confidence: string;
      oneLineWhy: string;
    }
  >();
  for (const d of decisions) {
    if (d.targetId === null) continue;
    const key = `${d.targetType}:${d.targetId}`;
    if (decisionByTarget.has(key)) continue;
    decisionByTarget.set(key, {
      category: d.category,
      confidence: d.confidence,
      oneLineWhy: d.oneLineWhy,
    });
  }

  // Resolve submission titles for appeal targets.
  const subTargetIds = rows
    .filter((r) => r.targetType === "submission")
    .map((r) => r.targetId);
  const subRows =
    subTargetIds.length > 0
      ? await db
          .select({ id: submissions.id, title: submissions.title })
          .from(submissions)
          .where(inArray(submissions.id, subTargetIds))
      : [];
  const subTitleById = new Map(subRows.map((r) => [r.id, r.title]));

  return rows.map((r) => {
    const drill = decisionByTarget.get(`${r.targetType}:${r.targetId}`);
    return {
      kind: "appeal" as const,
      id: r.id,
      createdAt: r.createdAt,
      reporter: r.reporterUsername,
      // Strip the "appeal: " prefix from the reason for cleaner
      // display; lib/appeals.ts inserts that prefix verbatim.
      reasonBody: r.reason.replace(/^appeal:\s*/, ""),
      targetType: r.targetType,
      targetId: r.targetId,
      targetTitle: subTitleById.get(r.targetId) ?? null,
      decisionCategory: drill?.category ?? null,
      decisionConfidence: drill?.confidence ?? null,
      decisionOneLineWhy: drill?.oneLineWhy ?? null,
    };
  });
}
