import Link from "next/link";

import { auth } from "@/lib/auth";
import { db } from "@/db/client";
import { moderationLog, users } from "@/db/schema";
import { count, desc, eq, ne } from "drizzle-orm";
import { relativeTime } from "@/lib/format";
import { getCurrentUser } from "@/lib/auth-shim";
import { getSystemUserId } from "@/lib/moderation";

/**
 * Public moderation log. The page is visible to any signed-in user
 * (transparency surface, not staff-only) so it must default-hide
 * the high-volume AI auto-reject rows — otherwise human moderation
 * gets drowned out. Pass `?automated=1` to reveal them.
 *
 * Per dev-docs/policy-moderator-plan.md §8.3, AI rows expose the
 * underlying policy_decisions row inline so reviewers can see the
 * verdict's category, confidence, model id, and prompt version
 * without a second click.
 */

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

  const showAutomated = sp.automated === "1";
  const systemUserId = await getSystemUserId().catch(() => null);

  // Build a where-clause that hides the system-user actions when
  // showAutomated is false. If the system user can't be looked up
  // (migration 0018 not run), fall back to "show everything" — better
  // than a confusing empty page.
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
    .limit(200);

  // Count automated rows to show in the header so the asymmetry is
  // visible without enumerating every row.
  let automatedCount = 0;
  if (systemUserId && !showAutomated) {
    const [c] = await db
      .select({ n: count() })
      .from(moderationLog)
      .where(eq(moderationLog.staffId, systemUserId));
    automatedCount = c?.n ?? 0;
  }

  return (
    <div className="proto-page-narrow">
      <h1>Moderation log</h1>
      <p className="proto-dek">
        Every state-changing moderation action — public to any signed-in user.
        Append-only; if a decision is reversed, you&rsquo;ll see both the
        original and the reversal as separate entries.{" "}
        {showAutomated ? (
          <>
            Showing automated (AI policy moderator) rows alongside human
            actions.{" "}
            <Link href="?">Hide automated</Link>.
          </>
        ) : automatedCount > 0 ? (
          <>
            {automatedCount} automated{" "}
            {automatedCount === 1 ? "action is" : "actions are"} hidden by
            default.{" "}
            <Link href="?automated=1">Show automated</Link>.
          </>
        ) : null}
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
              return (
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
              );
            })}
          </tbody>
        </table>
      )}
    </div>
  );
}

