import Link from "next/link";
import { and, desc, eq, isNull } from "drizzle-orm";

import { db } from "@/db/client";
import { flags, submissions, users } from "@/db/schema";
import { relativeTime } from "@/lib/format";
import { ModButton } from "@/components/prototype/admin/ModButton";
import { staffGate } from "@/lib/staff-gate";

export default async function ModQueue({
  searchParams,
}: {
  searchParams: Promise<{ as?: string }>;
}) {
  const sp = await searchParams;
  const gate = await staffGate(sp);
  if (gate) return gate;

  // Lane 1: open flags
  const openFlags = await db
    .select({
      id: flags.id,
      targetType: flags.targetType,
      targetId: flags.targetId,
      reason: flags.reason,
      createdAt: flags.createdAt,
      reporterUsername: users.username,
    })
    .from(flags)
    .innerJoin(users, eq(users.id, flags.reporterId))
    .where(eq(flags.status, "open"))
    .orderBy(desc(flags.createdAt))
    .limit(50);

  // Lane 2: first-submission queue (pending submissions)
  const pendingSubs = await db
    .select({
      id: submissions.id,
      title: submissions.title,
      url: submissions.url,
      type: submissions.type,
      createdAt: submissions.createdAt,
      authorUsername: users.username,
    })
    .from(submissions)
    .innerJoin(users, eq(users.id, submissions.authorId))
    .where(
      and(eq(submissions.state, "pending"), isNull(submissions.deletedAt)),
    )
    .orderBy(desc(submissions.createdAt))
    .limit(50);

  return (
    <section>
      <h2>Review queue</h2>
      <p className="proto-dek">
        Items reported by users (open flags) and first-time submissions
        awaiting staff review. Acting on any item appends a row to{" "}
        <Link href="/admin/log">/admin/log</Link>.
      </p>

      <h3 className="proto-h3">Open flags ({openFlags.length})</h3>
      {openFlags.length === 0 ? (
        <p className="proto-empty">No open flags.</p>
      ) : (
        <table className="proto-mod-table">
          <thead>
            <tr>
              <th>When</th>
              <th>Reporter</th>
              <th>Target</th>
              <th>Reason</th>
              <th>Action</th>
            </tr>
          </thead>
          <tbody>
            {openFlags.map((f) => {
              const targetHref =
                f.targetType === "submission" ? `/post/${f.targetId}` : "#";
              return (
                <tr key={f.id}>
                  <td>{relativeTime(f.createdAt.toISOString())}</td>
                  <td>@{f.reporterUsername}</td>
                  <td>
                    <Link href={targetHref}>
                      {f.targetType} · {f.targetId.slice(0, 8)}…
                    </Link>
                  </td>
                  <td className="proto-mod-reason">{f.reason}</td>
                  <td className="proto-mod-actions">
                    <ModButton
                      action="dismiss_flag"
                      targetId={f.targetId}
                      flagId={f.id}
                      className="proto-mod-btn proto-mod-btn-keep"
                      pendingLabel="Dismissing…"
                    >
                      Dismiss
                    </ModButton>
                    <ModButton
                      action="delete"
                      targetType={f.targetType}
                      targetId={f.targetId}
                      flagId={f.id}
                      className="proto-mod-btn proto-mod-btn-remove"
                      pendingLabel="Removing…"
                    >
                      Remove target
                    </ModButton>
                  </td>
                </tr>
              );
            })}
          </tbody>
        </table>
      )}

      <h3 className="proto-h3">
        First-submission queue ({pendingSubs.length})
      </h3>
      {pendingSubs.length === 0 ? (
        <p className="proto-empty">No pending submissions.</p>
      ) : (
        <table className="proto-mod-table">
          <thead>
            <tr>
              <th>When</th>
              <th>Author</th>
              <th>Type</th>
              <th>Title</th>
              <th>Action</th>
            </tr>
          </thead>
          <tbody>
            {pendingSubs.map((s) => (
              <tr key={s.id}>
                <td>{relativeTime(s.createdAt.toISOString())}</td>
                <td>@{s.authorUsername}</td>
                <td>
                  <span className="proto-mod-target-type">{s.type}</span>
                </td>
                <td>
                  {s.url ? (
                    <a href={s.url} target="_blank" rel="noopener noreferrer">
                      {s.title}
                    </a>
                  ) : (
                    s.title
                  )}
                </td>
                <td className="proto-mod-actions">
                  <ModButton
                    action="approve"
                    targetType="submission"
                    targetId={s.id}
                    className="proto-mod-btn proto-mod-btn-keep"
                    pendingLabel="Approving…"
                  >
                    Approve
                  </ModButton>
                  <ModButton
                    action="reject"
                    targetType="submission"
                    targetId={s.id}
                    className="proto-mod-btn proto-mod-btn-remove"
                    pendingLabel="Rejecting…"
                  >
                    Reject
                  </ModButton>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      )}
    </section>
  );
}
