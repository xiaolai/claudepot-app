import Link from "next/link";
import { and, desc, eq, inArray, like } from "drizzle-orm";

import { db } from "@/db/client";
import { flags, policyDecisions, submissions, users } from "@/db/schema";
import { staffGate } from "@/lib/staff-gate";
import { relativeTime } from "@/lib/format";
import { ModButton } from "@/components/prototype/admin/ModButton";

const PAGE_LIMIT = 100;

/**
 * /admin/console/appeals — open appeals on AI rejects, joined with
 * the originating policy_decisions row so the operator can see
 * what Ada said and what the user is contesting in one view.
 *
 * Appeals share the `flags` table with community flags but use a
 * `reason LIKE 'appeal:%'` convention. lib/appeals.ts inserts
 * them with the user's narrative after the prefix. The
 * one-appeal-per-target uniqueness constraint lives on
 * `flags`'s partial unique index `idx_flags_open_appeal_per_target`.
 */
export default async function AppealsPage({
  searchParams,
}: {
  searchParams: Promise<{ as?: string }>;
}) {
  const sp = await searchParams;
  const gate = await staffGate(sp);
  if (gate) return gate;

  const openAppeals = await db
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
    .limit(PAGE_LIMIT);

  const targetIds = openAppeals.map((a) => a.targetId);
  const subTargetIds = openAppeals
    .filter((a) => a.targetType === "submission")
    .map((a) => a.targetId);

  const [decisions, subRows] = await Promise.all([
    targetIds.length > 0
      ? db
          .select({
            targetType: policyDecisions.targetType,
            targetId: policyDecisions.targetId,
            decisionId: policyDecisions.id,
            category: policyDecisions.category,
            confidence: policyDecisions.confidence,
            oneLineWhy: policyDecisions.oneLineWhy,
            modelId: policyDecisions.modelId,
            promptVersion: policyDecisions.promptVersion,
            decidedAt: policyDecisions.decidedAt,
          })
          .from(policyDecisions)
          .where(inArray(policyDecisions.targetId, targetIds))
          .orderBy(desc(policyDecisions.decidedAt))
      : Promise.resolve(
          [] as Array<{
            targetType: string | null;
            targetId: string | null;
            decisionId: string;
            category: string | null;
            confidence: string;
            oneLineWhy: string;
            modelId: string;
            promptVersion: string;
            decidedAt: Date;
          }>,
        ),
    subTargetIds.length > 0
      ? db
          .select({ id: submissions.id, title: submissions.title })
          .from(submissions)
          .where(inArray(submissions.id, subTargetIds))
      : Promise.resolve([] as Array<{ id: string; title: string }>),
  ]);

  // Walk decisions in DESC order, keep first hit per (type, id).
  const decisionByTarget = new Map<
    string,
    {
      decisionId: string;
      category: string | null;
      confidence: string;
      oneLineWhy: string;
      modelId: string;
      promptVersion: string;
      decidedAt: Date;
    }
  >();
  for (const d of decisions) {
    if (d.targetId === null) continue;
    const key = `${d.targetType}:${d.targetId}`;
    if (decisionByTarget.has(key)) continue;
    decisionByTarget.set(key, {
      decisionId: d.decisionId,
      category: d.category,
      confidence: d.confidence,
      oneLineWhy: d.oneLineWhy,
      modelId: d.modelId,
      promptVersion: d.promptVersion,
      decidedAt: d.decidedAt,
    });
  }
  const subTitleById = new Map(subRows.map((r) => [r.id, r.title]));
  const asSuffix = sp.as ? `?as=${sp.as}` : "";

  return (
    <section>
      <div className="proto-console-breadcrumb">
        <Link href={`/admin/console${asSuffix}`}>← Console</Link>
      </div>
      <h2>Appeals</h2>
      <p className="proto-dek">
        Authors challenging an AI reject. <strong>{openAppeals.length}</strong>{" "}
        open. Uphold leaves the decision in place; restore reverses
        it (and writes a <code>restore</code> row to{" "}
        <code>moderation_log</code>). Both close the flag.
      </p>

      {openAppeals.length === 0 ? (
        <p className="proto-empty proto-empty-spaced">
          No open appeals.
        </p>
      ) : (
        <div className="proto-appeals-list">
          {openAppeals.map((a) => {
            const drill = decisionByTarget.get(`${a.targetType}:${a.targetId}`);
            const isSubmission = a.targetType === "submission";
            const targetTitle = isSubmission
              ? (subTitleById.get(a.targetId) ?? `submission ${a.targetId.slice(0, 8)}…`)
              : `${a.targetType} ${a.targetId.slice(0, 8)}…`;
            const targetHref = isSubmission ? `/post/${a.targetId}` : "#";
            const reasonBody = a.reason.replace(/^appeal:\s*/, "");
            return (
              <article key={a.id} className="proto-appeal">
                <header className="proto-appeal-head">
                  <span className="proto-appeal-time">
                    {relativeTime(a.createdAt.toISOString())}
                  </span>
                  <Link
                    href={`/u/${a.reporterUsername}`}
                    className="proto-appeal-user"
                  >
                    @{a.reporterUsername}
                  </Link>
                  <span className="proto-appeal-arrow">vs</span>
                  <span className="proto-appeal-system">Ada</span>
                </header>

                <div className="proto-appeal-body">
                  <div className="proto-appeal-target">
                    {targetHref === "#" ? (
                      <span>{targetTitle}</span>
                    ) : (
                      <Link href={targetHref}>{targetTitle}</Link>
                    )}
                  </div>

                  {drill ? (
                    <div className="proto-appeal-decision">
                      <span className="proto-state-pill proto-state-pill-rejected">
                        Ada rejected · {drill.category ?? "—"}
                      </span>{" "}
                      <span className="proto-meta-quiet">
                        conf={drill.confidence} · {drill.modelId} · v
                        {drill.promptVersion} ·{" "}
                        {relativeTime(drill.decidedAt.toISOString())}
                      </span>
                      <p className="proto-appeal-why">
                        <strong>Ada said:</strong>{" "}
                        <em>{drill.oneLineWhy}</em>
                      </p>
                    </div>
                  ) : (
                    <p className="proto-meta-quiet">
                      No matching decision row — the decision may have
                      been deleted or the appeal predates the
                      pipeline. Restore is still safe; uphold leaves
                      the target as-is.
                    </p>
                  )}

                  <p className="proto-appeal-narrative">
                    <strong>{`@${a.reporterUsername} said:`}</strong>{" "}
                    {reasonBody}
                  </p>
                </div>

                <div className="proto-appeal-actions">
                  <ModButton
                    action="dismiss_flag"
                    targetId={a.targetId}
                    flagId={a.id}
                    className="proto-mod-btn proto-mod-btn-keep"
                    pendingLabel="Upholding…"
                  >
                    Uphold reject
                  </ModButton>
                  {a.targetType === "user" ? (
                    <span className="proto-meta-quiet">
                      User-targeted appeal — staff dismisses; no
                      restore action applies.
                    </span>
                  ) : (
                    <ModButton
                      action="restore"
                      targetType={a.targetType === "comment" ? "comment" : "submission"}
                      targetId={a.targetId}
                      flagId={a.id}
                      className="proto-mod-btn proto-mod-btn-remove"
                      pendingLabel="Restoring…"
                    >
                      Restore
                    </ModButton>
                  )}
                </div>
              </article>
            );
          })}
        </div>
      )}
    </section>
  );
}
