import Link from "next/link";
import { and, count, desc, eq, gte, inArray, ne } from "drizzle-orm";

import { db } from "@/db/client";
import { moderationLog, policyDecisions, submissions } from "@/db/schema";
import { staffGate } from "@/lib/staff-gate";
import { relativeTime } from "@/lib/format";
import { getSystemUserId } from "@/lib/moderation";

const PAGE_LIMIT = 100;

/**
 * /admin/console/decisions — staff-only, real-data view of every
 * AI moderation decision.
 *
 * Replaces the fixture-only /admin/audit page from the pre-redesign
 * tab grid. Reads `policy_decisions` directly. Joins to
 * `submissions` for titles where the target is a submission;
 * comments and users render the truncated id (link target lives on
 * `/admin/decision/[id]` once it ships in phase 4).
 *
 * Override detection: a decision is marked overridden if its
 * (target_type, target_id) appears in moderation_log with a
 * `restore` action by a non-system staff after the decision time.
 * Restores by Ada herself (retro queue) don't count.
 */
export default async function DecisionsPage({
  searchParams,
}: {
  searchParams: Promise<{ as?: string; verdict?: string }>;
}) {
  const sp = await searchParams;
  const gate = await staffGate(sp);
  if (gate) return gate;

  const verdictFilter =
    sp.verdict === "pass" || sp.verdict === "reject" ? sp.verdict : null;
  const sevenDaysAgo = new Date(Date.now() - 7 * 24 * 60 * 60 * 1000);
  const systemUserId = await getSystemUserId().catch(() => null);

  // Last 7 days of rejects, total count of rejects, and recent
  // decisions (filtered) — three queries in parallel.
  const [recentDecisions, sevenDayRejects, sevenDayRestores] =
    await Promise.all([
      db
        .select({
          id: policyDecisions.id,
          decidedAt: policyDecisions.decidedAt,
          targetType: policyDecisions.targetType,
          targetId: policyDecisions.targetId,
          verdict: policyDecisions.verdict,
          category: policyDecisions.category,
          confidence: policyDecisions.confidence,
          oneLineWhy: policyDecisions.oneLineWhy,
          modelId: policyDecisions.modelId,
          promptVersion: policyDecisions.promptVersion,
          passNumber: policyDecisions.passNumber,
        })
        .from(policyDecisions)
        .where(
          verdictFilter
            ? eq(policyDecisions.verdict, verdictFilter)
            : undefined,
        )
        .orderBy(desc(policyDecisions.decidedAt))
        .limit(PAGE_LIMIT),
      db
        .select({
          targetType: policyDecisions.targetType,
          targetId: policyDecisions.targetId,
        })
        .from(policyDecisions)
        .where(
          and(
            eq(policyDecisions.verdict, "reject"),
            gte(policyDecisions.decidedAt, sevenDaysAgo),
          ),
        ),
      systemUserId
        ? db
            .select({
              targetType: moderationLog.targetType,
              targetId: moderationLog.targetId,
            })
            .from(moderationLog)
            .where(
              and(
                eq(moderationLog.action, "restore"),
                ne(moderationLog.staffId, systemUserId),
                gte(moderationLog.createdAt, sevenDaysAgo),
              ),
            )
        : Promise.resolve(
            [] as Array<{ targetType: string | null; targetId: string | null }>,
          ),
    ]);

  // Override-set keyed by `${type}:${id}`.
  const restoredKeys = new Set(
    sevenDayRestores
      .filter((r) => r.targetType && r.targetId)
      .map((r) => `${r.targetType}:${r.targetId}`),
  );
  const overriddenIn7d = sevenDayRejects.filter((d) =>
    restoredKeys.has(`${d.targetType}:${d.targetId}`),
  ).length;
  const agreement7d =
    sevenDayRejects.length === 0
      ? null
      : Math.round(
          ((sevenDayRejects.length - overriddenIn7d) / sevenDayRejects.length) *
            100,
        );

  // Resolve submission titles for the page in one round-trip.
  const subTargetIds = recentDecisions
    .filter((d) => d.targetType === "submission" && d.targetId !== null)
    .map((d) => d.targetId!) as string[];
  const subRows =
    subTargetIds.length > 0
      ? await db
          .select({ id: submissions.id, title: submissions.title })
          .from(submissions)
          .where(inArray(submissions.id, subTargetIds))
      : [];
  const subTitleById = new Map(subRows.map((r) => [r.id, r.title]));

  // Total count of decisions for the dek line. One COUNT query is
  // cheap enough at solo scale; expose it so the operator knows
  // the page is showing a slice.
  const [{ n: totalCount } = { n: 0 }] = await db
    .select({ n: count() })
    .from(policyDecisions);

  const asSuffix = sp.as ? `?as=${sp.as}` : "";

  return (
    <section>
      <div className="proto-console-breadcrumb">
        <Link href={`/admin/console${asSuffix}`}>← Console</Link>
      </div>
      <h2>Decisions</h2>
      <p className="proto-dek">
        Every AI moderation decision, recent first. {totalCount} total ·{" "}
        showing {recentDecisions.length}.
        {agreement7d !== null ? (
          <>
            {" "}
            Ada agreement (7d): <strong>{agreement7d}%</strong>{" "}
            ({sevenDayRejects.length - overriddenIn7d}/
            {sevenDayRejects.length} rejects upheld).
          </>
        ) : null}
      </p>
      <p className="proto-dek">
        Verdict:{" "}
        <Link
          href={verdictFilter === null ? "#" : `/admin/console/decisions${asSuffix}`}
          aria-current={verdictFilter === null ? "page" : undefined}
        >
          all
        </Link>{" "}
        ·{" "}
        <Link
          href={`/admin/console/decisions?verdict=reject${asSuffix.replace("?", "&")}`}
          aria-current={verdictFilter === "reject" ? "page" : undefined}
        >
          reject
        </Link>{" "}
        ·{" "}
        <Link
          href={`/admin/console/decisions?verdict=pass${asSuffix.replace("?", "&")}`}
          aria-current={verdictFilter === "pass" ? "page" : undefined}
        >
          pass
        </Link>
      </p>

      {recentDecisions.length === 0 ? (
        <p className="proto-empty proto-empty-spaced">
          No decisions yet. The pipeline writes a row to{" "}
          <code>policy_decisions</code> on every submission and comment.
        </p>
      ) : (
        <table className="proto-mod-table">
          <thead>
            <tr>
              <th>Decided</th>
              <th>Target</th>
              <th>Verdict</th>
              <th>Confidence</th>
              <th>Model · pass</th>
              <th>One-line why</th>
            </tr>
          </thead>
          <tbody>
            {recentDecisions.map((d) => {
              const isSubmission = d.targetType === "submission";
              const targetLabel =
                isSubmission && d.targetId
                  ? (subTitleById.get(d.targetId) ?? `submission ${d.targetId.slice(0, 8)}…`)
                  : d.targetId
                    ? `${d.targetType} ${d.targetId.slice(0, 8)}…`
                    : `${d.targetType} (deleted)`;
              const targetHref =
                isSubmission && d.targetId ? `/post/${d.targetId}` : "#";
              const verdictPill =
                d.verdict === "reject"
                  ? "proto-state-pill proto-state-pill-rejected"
                  : "proto-state-pill proto-state-pill-approved";
              return (
                <tr key={d.id}>
                  <td>{relativeTime(d.decidedAt.toISOString())}</td>
                  <td>
                    {targetHref === "#" ? (
                      <span className="proto-meta-quiet">{targetLabel}</span>
                    ) : (
                      <Link href={targetHref}>{targetLabel}</Link>
                    )}
                  </td>
                  <td>
                    <span className={verdictPill}>
                      {d.verdict}
                      {d.category ? ` · ${d.category}` : null}
                    </span>
                  </td>
                  <td>{d.confidence}</td>
                  <td>
                    <code>
                      {d.modelId} · v{d.promptVersion} · pass {d.passNumber}
                    </code>
                  </td>
                  <td className="proto-mod-reason">{d.oneLineWhy}</td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}
    </section>
  );
}
