import type { ReactNode } from "react";
import Link from "next/link";
import { Check, Hash, X } from "lucide-react";
import { getAuditLog } from "@/lib/prototype-fixtures";
import { relativeTime } from "@/lib/format";
import { staffGate } from "@/lib/staff-gate";
import { getSubmissionById } from "@/db/queries";

function ActionLabel({
  icon: Icon,
  children,
}: {
  icon: typeof Check;
  children: ReactNode;
}) {
  return (
    <>
      <span className="proto-inline-icon" aria-hidden>
        <Icon size={12} />
      </span>{" "}
      {children}
    </>
  );
}

const ACTION_LABELS: Record<string, ReactNode> = {
  approve: <ActionLabel icon={Check}>approved</ActionLabel>,
  reject: <ActionLabel icon={X}>rejected</ActionLabel>,
  tag: <ActionLabel icon={Hash}>tagged</ActionLabel>,
};

export default async function AuditLog({
  searchParams,
}: {
  searchParams: Promise<{ as?: string }>;
}) {
  const sp = await searchParams;
  const gate = await staffGate(sp);
  if (gate) return gate;

  const log = getAuditLog();
  const overrides = log.filter((e) => e.overridden).length;
  // No decisions yet → render "—" rather than NaN%. The percentage
  // only carries meaning with a non-empty denominator.
  const agreement =
    log.length === 0 ? null : ((log.length - overrides) / log.length) * 100;
  const targetSubs = await Promise.all(
    log.map((e) =>
      e.target_type === "submission"
        ? getSubmissionById(e.target_id)
        : Promise.resolve(undefined),
    ),
  );

  return (
    <section>
      <h2>AI audit log</h2>
      <p className="proto-dek">
        Every AI moderation decision, recent first. Agreement rate this
        window:{" "}
        <strong>
          {agreement === null ? "—" : `${agreement.toFixed(0)}%`}
        </strong>
        .
      </p>
      <p className="proto-empty proto-empty-spaced">
        Read-only. The decision pipeline isn&rsquo;t wired up yet — entries
        come from fixture data. Override will land when the pipeline writes
        to a real <code>ai_decisions</code> table.
      </p>

      <table className="proto-mod-table">
        <thead>
          <tr>
            <th>Decision</th>
            <th>Target</th>
            <th>Reason</th>
            <th>Confidence</th>
            <th>When</th>
            <th>Override</th>
          </tr>
        </thead>
        <tbody>
          {log.map((e, idx) => {
            const isSubmission = e.target_type === "submission";
            const sub = isSubmission ? targetSubs[idx] : null;
            const targetLabel = sub
              ? sub.title
              : `${e.target_type} ${e.target_id}`;
            const targetHref = isSubmission ? `/post/${e.target_id}` : "#";
            return (
              <tr key={e.id} className={e.overridden ? "proto-mod-row-override" : undefined}>
                <td>
                  <span
                    className={`proto-state-pill proto-state-pill-${e.action === "approve" ? "pending" : "rejected"}`}
                  >
                    {ACTION_LABELS[e.action]}
                  </span>
                </td>
                <td>
                  <Link href={targetHref}>{targetLabel}</Link>
                </td>
                <td className="proto-mod-reason">{e.reason}</td>
                <td>{Math.round(e.confidence * 100)}%</td>
                <td>{relativeTime(e.decided_at)}</td>
                <td>
                  {e.overridden ? (
                    <span>
                      <strong>@{e.overridden.by}</strong> →{" "}
                      {e.overridden.new_action} · {relativeTime(e.overridden.at)}
                      {e.overridden.note && (
                        <div className="proto-mod-target-type">
                          {e.overridden.note}
                        </div>
                      )}
                    </span>
                  ) : (
                    <span className="proto-meta-quiet">—</span>
                  )}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </section>
  );
}
