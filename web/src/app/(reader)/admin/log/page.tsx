import Link from "next/link";

import { auth } from "@/lib/auth";
import { db } from "@/db/client";
import { moderationLog, users } from "@/db/schema";
import { desc, eq } from "drizzle-orm";
import { relativeTime } from "@/lib/format";
import { getCurrentUser } from "@/lib/auth-shim";

export default async function ModLog({
  searchParams,
}: {
  searchParams: Promise<{ as?: string }>;
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

  const rows = await db
    .select({
      id: moderationLog.id,
      action: moderationLog.action,
      targetType: moderationLog.targetType,
      targetId: moderationLog.targetId,
      note: moderationLog.note,
      createdAt: moderationLog.createdAt,
      staffUsername: users.username,
    })
    .from(moderationLog)
    .innerJoin(users, eq(users.id, moderationLog.staffId))
    .orderBy(desc(moderationLog.createdAt))
    .limit(200);

  return (
    <div className="proto-page-narrow">
      <h1>Moderation log</h1>
      <p className="proto-dek">
        Every staff action — public to any signed-in user. Append-only; if a
        decision is reversed, you'll see both the original and the reversal as
        separate entries.
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
            {rows.map((r) => (
              <tr key={r.id}>
                <td>{relativeTime(r.createdAt.toISOString())}</td>
                <td>
                  <Link href={`/u/${r.staffUsername}`}>@{r.staffUsername}</Link>
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
            ))}
          </tbody>
        </table>
      )}
    </div>
  );
}
