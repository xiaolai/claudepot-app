import Link from "next/link";
import { desc, sql } from "drizzle-orm";

import { db } from "@/db/client";
import { submissions, users } from "@/db/schema";
import { relativeTime } from "@/lib/format";
import { ModButton } from "@/components/prototype/admin/ModButton";
import { staffGate } from "@/lib/staff-gate";

export default async function AdminUsers({
  searchParams,
}: {
  searchParams: Promise<{ as?: string }>;
}) {
  const sp = await searchParams;
  const gate = await staffGate(sp);
  if (gate) return gate;

  // Direct query so we keep the DB id alongside the display fields.
  // The shared User type from prototype-fixtures.ts is fixture-shaped
  // and intentionally drops `id`; the admin page is the one place that
  // needs both, so we read directly here. Post counts come from a
  // grouped subquery instead of N+1 round-trips.
  const rows = await db
    .select({
      id: users.id,
      username: users.username,
      displayName: users.name,
      karma: users.karma,
      role: users.role,
      createdAt: users.createdAt,
      posts: sql<number>`(
        SELECT COUNT(*)::int FROM ${submissions}
        WHERE ${submissions.authorId} = ${users.id}
          AND ${submissions.deletedAt} IS NULL
      )`,
    })
    .from(users)
    .orderBy(desc(users.karma));

  return (
    <section>
      <h2>Users</h2>
      <p className="proto-dek">
        {rows.length} accounts. Staff and system users can auto-post.
        Suspending sets <code>role = locked</code> and revokes all live
        sessions; the user can no longer sign in until reinstated.
      </p>

      <table className="proto-mod-table">
        <thead>
          <tr>
            <th>User</th>
            <th>Karma</th>
            <th>Joined</th>
            <th>Posts</th>
            <th>Role</th>
            <th>Action</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((u) => {
            const isLocked = u.role === "locked";
            return (
              <tr key={u.username}>
                <td>
                  <Link href={`/u/${u.username}`}>@{u.username}</Link>
                  {u.displayName && u.displayName !== u.username ? (
                    <span className="proto-mod-target-type"> · {u.displayName}</span>
                  ) : null}
                </td>
                <td>{u.karma}</td>
                <td>{relativeTime(u.createdAt.toISOString())}</td>
                <td>{u.posts}</td>
                <td>
                  <span className="proto-state-pill proto-state-pill-pending">
                    {u.role}
                  </span>
                </td>
                <td className="proto-mod-actions">
                  <Link
                    href={`/u/${u.username}`}
                    className="proto-mod-btn proto-mod-btn-keep"
                  >
                    View
                  </Link>
                  {isLocked ? (
                    <span className="proto-meta-quiet">suspended</span>
                  ) : (
                    <ModButton
                      action="lock_user"
                      targetId={u.id}
                      className="proto-mod-btn proto-mod-btn-remove"
                      pendingLabel="Suspending…"
                    >
                      Suspend
                    </ModButton>
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
