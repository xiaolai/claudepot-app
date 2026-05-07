import { notFound } from "next/navigation";
import Link from "next/link";
import { and, desc, eq } from "drizzle-orm";

import { db } from "@/db/client";
import {
  comments,
  moderationLog,
  policyDecisions,
  submissions,
  users,
} from "@/db/schema";
import { staffGate } from "@/lib/staff-gate";
import { relativeTime } from "@/lib/format";
import { ModButton } from "@/components/prototype/admin/ModButton";
import { getSystemUserId } from "@/lib/moderation";

/**
 * /admin/decision/[id] — single AI decision drill page.
 *
 * Reached from /admin/console/decisions or by deep link. Shows
 * the full Ada output, the target's content snapshot, the
 * override history (any restore/lock/delete by staff after the
 * decision), and an inline restore action when applicable.
 *
 * UUID validation is server-side: a malformed id raises notFound()
 * rather than handing it to Drizzle. The 404 is intentional — the
 * page exists, the row does not.
 */
export default async function DecisionDrillPage({
  params,
  searchParams,
}: {
  params: Promise<{ id: string }>;
  searchParams: Promise<{ as?: string }>;
}) {
  const { id } = await params;
  const sp = await searchParams;
  const gate = await staffGate(sp);
  if (gate) return gate;

  if (!isUuid(id)) notFound();

  const [decision] = await db
    .select({
      id: policyDecisions.id,
      authorId: policyDecisions.authorId,
      targetType: policyDecisions.targetType,
      targetId: policyDecisions.targetId,
      verdict: policyDecisions.verdict,
      category: policyDecisions.category,
      confidence: policyDecisions.confidence,
      oneLineWhy: policyDecisions.oneLineWhy,
      modelId: policyDecisions.modelId,
      promptVersion: policyDecisions.promptVersion,
      passNumber: policyDecisions.passNumber,
      costUsd: policyDecisions.costUsd,
      decidedAt: policyDecisions.decidedAt,
    })
    .from(policyDecisions)
    .where(eq(policyDecisions.id, id))
    .limit(1);

  if (!decision) notFound();

  const [authorRow] = await db
    .select({ username: users.username })
    .from(users)
    .where(eq(users.id, decision.authorId))
    .limit(1);

  // Target snapshot — submission or comment. User-targeted
  // decisions don't carry a content snapshot here; they're shown
  // with just the username.
  const [targetSubmission, targetComment, targetUser] = await Promise.all([
    decision.targetType === "submission" && decision.targetId
      ? db
          .select({
            id: submissions.id,
            title: submissions.title,
            url: submissions.url,
            text: submissions.text,
            type: submissions.type,
            state: submissions.state,
          })
          .from(submissions)
          .where(eq(submissions.id, decision.targetId))
          .limit(1)
          .then((rows) => rows[0] ?? null)
      : Promise.resolve(null),
    decision.targetType === "comment" && decision.targetId
      ? db
          .select({
            id: comments.id,
            body: comments.body,
            state: comments.state,
            submissionId: comments.submissionId,
          })
          .from(comments)
          .where(eq(comments.id, decision.targetId))
          .limit(1)
          .then((rows) => rows[0] ?? null)
      : Promise.resolve(null),
    decision.targetType === "user" && decision.targetId
      ? db
          .select({ username: users.username })
          .from(users)
          .where(eq(users.id, decision.targetId))
          .limit(1)
          .then((rows) => rows[0] ?? null)
      : Promise.resolve(null),
  ]);

  // Override history — every moderation_log row for this target
  // since the decision time. Joined to staff usernames so we can
  // render the full audit trail.
  const overrides = decision.targetId
    ? await db
        .select({
          id: moderationLog.id,
          action: moderationLog.action,
          note: moderationLog.note,
          createdAt: moderationLog.createdAt,
          staffId: moderationLog.staffId,
          staffUsername: users.username,
        })
        .from(moderationLog)
        .innerJoin(users, eq(users.id, moderationLog.staffId))
        .where(
          and(
            eq(moderationLog.targetType, decision.targetType),
            eq(moderationLog.targetId, decision.targetId),
          ),
        )
        .orderBy(desc(moderationLog.createdAt))
    : [];

  const systemUserId = await getSystemUserId().catch(() => null);
  const restoredByHuman = overrides.some(
    (o) =>
      o.action === "restore" &&
      (!systemUserId || o.staffId !== systemUserId),
  );
  const targetState =
    targetSubmission?.state ?? targetComment?.state ?? null;
  const canRestore =
    decision.verdict === "reject" &&
    decision.targetType !== "user" &&
    decision.targetId !== null &&
    !restoredByHuman &&
    targetState !== null &&
    targetState !== "approved";

  const verdictPill =
    decision.verdict === "reject"
      ? "proto-state-pill proto-state-pill-rejected"
      : "proto-state-pill proto-state-pill-approved";

  const asSuffix = sp.as ? `?as=${sp.as}` : "";

  return (
    <section>
      <div className="proto-console-breadcrumb">
        <Link href={`/admin/console/decisions${asSuffix}`}>
          ← Decisions
        </Link>
      </div>
      <h2>Decision drill</h2>

      <article className="proto-decision">
        <header className="proto-decision-head">
          <span className={verdictPill}>
            {decision.verdict}
            {decision.category ? ` · ${decision.category}` : null}
          </span>
          <span className="proto-decision-conf">
            confidence: <code>{decision.confidence}</code>
          </span>
          <span className="proto-decision-time">
            {relativeTime(decision.decidedAt.toISOString())}
          </span>
        </header>

        <p className="proto-decision-why">
          <strong>One-line why:</strong> <em>{decision.oneLineWhy}</em>
        </p>

        <dl className="proto-decision-meta">
          <dt>Model</dt>
          <dd>
            <code>{decision.modelId}</code>
          </dd>
          <dt>Prompt version</dt>
          <dd>
            <code>{decision.promptVersion}</code>
          </dd>
          <dt>Pass</dt>
          <dd>{decision.passNumber}</dd>
          <dt>Cost</dt>
          <dd>
            {decision.costUsd ? (
              <>${Number(decision.costUsd).toFixed(6)}</>
            ) : (
              <span className="proto-meta-quiet">not recorded</span>
            )}
          </dd>
          <dt>Author</dt>
          <dd>
            {authorRow ? (
              <Link href={`/u/${authorRow.username}`}>
                @{authorRow.username}
              </Link>
            ) : (
              <span className="proto-meta-quiet">unknown</span>
            )}
          </dd>
          <dt>Decision id</dt>
          <dd>
            <code>{decision.id}</code>
          </dd>
        </dl>
      </article>

      <article className="proto-decision-target">
        <h3>Target</h3>
        {targetSubmission ? (
          <>
            <p className="proto-decision-target-meta">
              <Link href={`/post/${targetSubmission.id}`}>
                <strong>{targetSubmission.title}</strong>
              </Link>{" "}
              <span className="proto-meta-quiet">
                · {targetSubmission.type} · state={targetSubmission.state}
              </span>
            </p>
            {targetSubmission.url ? (
              <p className="proto-decision-target-url">
                <a
                  href={targetSubmission.url}
                  target="_blank"
                  rel="noopener noreferrer"
                >
                  {targetSubmission.url}
                </a>
              </p>
            ) : null}
            {targetSubmission.text ? (
              <pre className="proto-decision-target-body">
                {targetSubmission.text}
              </pre>
            ) : null}
          </>
        ) : targetComment ? (
          <>
            <p className="proto-decision-target-meta">
              comment on{" "}
              <Link href={`/post/${targetComment.submissionId}#comments`}>
                submission {targetComment.submissionId.slice(0, 8)}…
              </Link>{" "}
              <span className="proto-meta-quiet">
                · state={targetComment.state}
              </span>
            </p>
            <pre className="proto-decision-target-body">
              {targetComment.body}
            </pre>
          </>
        ) : targetUser ? (
          <p className="proto-decision-target-meta">
            <Link href={`/u/${targetUser.username}`}>
              @{targetUser.username}
            </Link>{" "}
            <span className="proto-meta-quiet">
              · user-targeted decision
            </span>
          </p>
        ) : (
          <p className="proto-meta-quiet">
            Target deleted or unavailable.
          </p>
        )}
      </article>

      <article className="proto-decision-history">
        <h3>Override history</h3>
        {overrides.length === 0 ? (
          <p className="proto-meta-quiet">
            No staff actions on this target yet.
          </p>
        ) : (
          <table className="proto-mod-table">
            <thead>
              <tr>
                <th>When</th>
                <th>By</th>
                <th>Action</th>
                <th>Note</th>
              </tr>
            </thead>
            <tbody>
              {overrides.map((o) => {
                const isAi = systemUserId && o.staffId === systemUserId;
                return (
                  <tr key={o.id}>
                    <td>{relativeTime(o.createdAt.toISOString())}</td>
                    <td>
                      {isAi ? (
                        <span className="proto-meta-quiet">
                          AI · {o.staffUsername}
                        </span>
                      ) : (
                        <Link href={`/u/${o.staffUsername}`}>
                          @{o.staffUsername}
                        </Link>
                      )}
                    </td>
                    <td>
                      <code>{o.action}</code>
                    </td>
                    <td className="proto-mod-reason">{o.note ?? ""}</td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
      </article>

      {canRestore ? (
        <article className="proto-decision-action">
          <h3>Override</h3>
          <p className="proto-dek">
            This decision is currently in effect. Restoring publishes
            the target and writes a <code>restore</code> row to{" "}
            <code>moderation_log</code>.
          </p>
          <ModButton
            action="restore"
            targetType={
              decision.targetType === "comment" ? "comment" : "submission"
            }
            targetId={decision.targetId!}
            className="proto-mod-btn proto-mod-btn-remove"
            pendingLabel="Restoring…"
          >
            Restore target
          </ModButton>
        </article>
      ) : null}
    </section>
  );
}

const UUID_REGEX =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

function isUuid(s: string): boolean {
  return UUID_REGEX.test(s);
}
