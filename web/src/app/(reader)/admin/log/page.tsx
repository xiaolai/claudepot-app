import Link from "next/link";

import { auth } from "@/lib/auth";
import { db } from "@/db/client";
import { moderationLog, policyDecisions, users } from "@/db/schema";
import { and, count, desc, eq, inArray, ne, sql } from "drizzle-orm";
import { relativeTime } from "@/lib/format";
import { getCurrentUser } from "@/lib/auth-shim";
import { getSystemUserId } from "@/lib/moderation";
import { requireStaffId } from "@/lib/staff";

/**
 * Public moderation log. The page is visible to any signed-in user
 * (transparency surface, not staff-only) so it must default-hide
 * the high-volume AI auto-reject rows — otherwise human moderation
 * gets drowned out. Pass `?automated=1` to reveal them.
 *
 * AI rows render a drill-down sub-row showing the policy_decisions
 * fields the public note hides (verdict category is the only thing
 * in the visible note for AI rejects, intentionally — the verbatim
 * one_line_why can quote the very PII the model just classified).
 * Staff readers see the model id, pass number, confidence, and
 * full one_line_why beneath each AI row. The drill-down is also
 * accessible to non-staff signed-in users; the one_line_why on
 * automated rows still doesn't render in the visible note column,
 * but the drill-down section is gated to staff for PII safety.
 *
 * Counts header shows BOTH staff-action and automated-action
 * totals so the asymmetry is visible regardless of which mode is on.
 */

const PAGE_LIMIT = 200;

export default async function ModLog({
  searchParams,
}: {
  searchParams: Promise<{ as?: string; automated?: string }>;
}) {
  const sp = await searchParams;
  const session = await auth();
  const devUser = getCurrentUser(sp);
  if (!session?.user && !devUser) {
    return (
      <div className="proto-page-narrow">
        <p className="proto-empty proto-empty-spaced">
          <Link href="/login">Sign in</Link> to read the moderation log.
        </p>
      </div>
    );
  }

  // Drill-down PII gate: only staff/system roles see the verbatim
  // one_line_why on AI rows. Other authed readers see the row but
  // the detail sub-row is suppressed.
  const viewerIsStaff = (await requireStaffId().catch(() => null)) !== null;

  const showAutomated = sp.automated === "1";
  const systemUserId = await getSystemUserId().catch(() => null);

  const hideAutomatedWhere =
    !showAutomated && systemUserId
      ? ne(moderationLog.staffId, systemUserId)
      : undefined;

  const rows = await db
    .select({
      id: moderationLog.id,
      action: moderationLog.action,
      targetType: moderationLog.targetType,
      targetId: moderationLog.targetId,
      note: moderationLog.note,
      createdAt: moderationLog.createdAt,
      staffUsername: users.username,
      staffId: moderationLog.staffId,
    })
    .from(moderationLog)
    .innerJoin(users, eq(users.id, moderationLog.staffId))
    .where(hideAutomatedWhere)
    .orderBy(desc(moderationLog.createdAt))
    .limit(PAGE_LIMIT);

  // Counts header. Both staff and automated counts are shown
  // regardless of which view is active so the user can see the
  // asymmetry without flipping the toggle.
  let staffCount = 0;
  let automatedCount = 0;
  if (systemUserId) {
    const [s] = await db
      .select({ n: count() })
      .from(moderationLog)
      .where(ne(moderationLog.staffId, systemUserId));
    staffCount = s?.n ?? 0;
    const [a] = await db
      .select({ n: count() })
      .from(moderationLog)
      .where(eq(moderationLog.staffId, systemUserId));
    automatedCount = a?.n ?? 0;
  } else {
    // No system user resolved — every row is a staff action.
    const [s] = await db.select({ n: count() }).from(moderationLog);
    staffCount = s?.n ?? 0;
  }

  // Drill-down lookup. For each AI row in `rows`, find the latest
  // matching policy_decisions row by (target_type, target_id). Done
  // as one query per page (not per row) — Postgres ranks by a
  // ROW_NUMBER() window so we get the latest decision per target.
  // Map keyed `${type}:${id}` for O(1) lookup at render time.
  type DrillRow = {
    decisionId: string;
    category: string | null;
    confidence: string;
    oneLineWhy: string;
    modelId: string;
    promptVersion: string;
    passNumber: number;
  };
  const drillByTarget = new Map<string, DrillRow>();
  if (systemUserId) {
    const aiTargets = rows
      .filter(
        (r) =>
          r.staffId === systemUserId &&
          r.targetType !== null &&
          r.targetId !== null,
      )
      .map((r) => r.targetId!) as string[];
    if (aiTargets.length > 0) {
      // The set of target_ids is small (at most PAGE_LIMIT = 200).
      // A flat IN over target_id then a JS-side group by
      // (type, id, pick latest) is cheaper than a CTE for this size.
      const decisions = await db
        .select({
          targetType: policyDecisions.targetType,
          targetId: policyDecisions.targetId,
          decisionId: policyDecisions.id,
          category: policyDecisions.category,
          confidence: policyDecisions.confidence,
          oneLineWhy: policyDecisions.oneLineWhy,
          modelId: policyDecisions.modelId,
          promptVersion: policyDecisions.promptVersion,
          passNumber: policyDecisions.passNumber,
          decidedAt: policyDecisions.decidedAt,
        })
        .from(policyDecisions)
        .where(
          and(
            // policy_decisions.targetId is nullable; the IN filter is
            // safe because we only fed in non-null targetIds above.
            inArray(policyDecisions.targetId, aiTargets),
          ),
        )
        .orderBy(desc(policyDecisions.decidedAt));
      // Walk in decided_at DESC order and keep the first hit per
      // (type, id) — that's the latest decision for the target.
      for (const d of decisions) {
        if (d.targetId === null) continue;
        const key = `${d.targetType}:${d.targetId}`;
        if (drillByTarget.has(key)) continue;
        drillByTarget.set(key, {
          decisionId: d.decisionId,
          category: d.category,
          confidence: d.confidence,
          oneLineWhy: d.oneLineWhy,
          modelId: d.modelId,
          promptVersion: d.promptVersion,
          passNumber: d.passNumber,
        });
      }
    }
    // Sanity: keep `sql` import used even if Drizzle inlines our SQL
    // somewhere else later. ESLint --no-unused isn't run here today;
    // touch it once so future tree-shake decisions don't drop it
    // silently.
    void sql;
  }

  return (
    <div className="proto-page-narrow">
      <h1>Moderation log</h1>
      <p className="proto-dek">
        Every state-changing moderation action — public to any signed-in user.
        Append-only; if a decision is reversed, you&rsquo;ll see both the
        original and the reversal as separate entries.
      </p>
      <p className="proto-dek">
        <strong>{staffCount}</strong> staff{" "}
        {staffCount === 1 ? "action" : "actions"} ·{" "}
        <strong>{automatedCount}</strong> automated{" "}
        {automatedCount === 1 ? "action" : "actions"}
        {showAutomated ? (
          <>
            {" "}
            (showing both){" "}
            <Link href="?">Hide automated</Link>.
          </>
        ) : (
          <>
            {" "}
            ({automatedCount === 0 ? "none" : `${automatedCount} hidden`}){" "}
            {automatedCount > 0 ? (
              <Link href="?automated=1">Show automated</Link>
            ) : null}
            .
          </>
        )}
      </p>

      {rows.length === 0 ? (
        <p className="proto-empty">No moderation actions yet.</p>
      ) : (
        <table className="proto-mod-table">
          <thead>
            <tr>
              <th>When</th>
              <th>By</th>
              <th>Action</th>
              <th>Target</th>
              <th>Note</th>
            </tr>
          </thead>
          <tbody>
            {rows.map((r) => {
              const isAutomated = systemUserId
                ? r.staffId === systemUserId
                : false;
              const drillKey =
                isAutomated && r.targetType && r.targetId
                  ? `${r.targetType}:${r.targetId}`
                  : null;
              const drill = drillKey ? drillByTarget.get(drillKey) : null;
              return (
                <>
                  <tr key={r.id}>
                    <td>{relativeTime(r.createdAt.toISOString())}</td>
                    <td>
                      {isAutomated ? (
                        <span className="proto-mod-target-type">
                          AI · {r.staffUsername}
                        </span>
                      ) : (
                        <Link href={`/u/${r.staffUsername}`}>
                          @{r.staffUsername}
                        </Link>
                      )}
                    </td>
                    <td>
                      <code>{r.action}</code>
                    </td>
                    <td>
                      {r.targetType ? (
                        <span className="proto-mod-target-type">
                          {r.targetType} · {r.targetId?.slice(0, 8)}…
                        </span>
                      ) : (
                        "—"
                      )}
                    </td>
                    <td className="proto-mod-reason">{r.note ?? ""}</td>
                  </tr>
                  {drill && viewerIsStaff ? (
                    <tr key={`${r.id}-drill`} className="proto-mod-drill">
                      <td />
                      <td colSpan={4} className="proto-mod-drill-detail">
                        <code>model={drill.modelId}</code>{" "}
                        <code>prompt_v={drill.promptVersion}</code>{" "}
                        <code>pass={drill.passNumber}</code>{" "}
                        <code>conf={drill.confidence}</code>
                        {drill.category ? (
                          <>
                            {" · "}
                            <strong>{drill.category}:</strong> {drill.oneLineWhy}
                          </>
                        ) : null}
                      </td>
                    </tr>
                  ) : null}
                </>
              );
            })}
          </tbody>
        </table>
      )}
    </div>
  );
}
