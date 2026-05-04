import type { ReactNode } from "react";
import Link from "next/link";
import { desc, eq, sql } from "drizzle-orm";
import { AtSign, Check, CornerDownRight } from "lucide-react";

import { db } from "@/db/client";
import { notifications, users } from "@/db/schema";
import { auth } from "@/lib/auth";
import { relativeTime } from "@/lib/format";
import { getCurrentUser } from "@/lib/auth-shim";
import { AccountSidebar } from "@/components/prototype/AccountSidebar";

function KindLabel({
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

const KIND_LABELS: Record<string, ReactNode> = {
  comment_reply: <KindLabel icon={CornerDownRight}>comment reply</KindLabel>,
  submission_reply: <KindLabel icon={CornerDownRight}>post reply</KindLabel>,
  moderation: <KindLabel icon={Check}>moderation</KindLabel>,
  mention: <KindLabel icon={AtSign}>mention</KindLabel>,
};

type NotePayload = {
  commentId?: string;
  submissionId?: string;
};

function buildLink(payload: unknown): string {
  const p = (payload ?? {}) as NotePayload;
  if (p.submissionId && p.commentId) {
    return `/post/${p.submissionId}#comment-${p.commentId}`;
  }
  if (p.submissionId) return `/post/${p.submissionId}`;
  return "#";
}

export default async function Notifications({
  searchParams,
}: {
  searchParams: Promise<{ as?: string }>;
}) {
  const sp = await searchParams;
  const session = await auth();
  const devUser = getCurrentUser(sp);

  // Real session takes precedence; fall back to the dev shim.
  let userId = session?.user?.id ?? null;
  let username = session?.user?.name ?? null;

  if (!userId && devUser) {
    const [u] = await db
      .select({ id: users.id, username: users.username })
      .from(users)
      .where(eq(users.username, devUser.username))
      .limit(1);
    if (u) {
      userId = u.id;
      username = u.username;
    }
  }

  if (!userId) {
    return (
      <div className="proto-page-narrow">
        <h1>Notifications</h1>
        <p className="proto-dek">
          <Link href="/login">Sign in</Link> to see notifications.
        </p>
      </div>
    );
  }

  const notes = await db
    .select({
      id: notifications.id,
      kind: notifications.kind,
      payload: notifications.payload,
      readAt: notifications.readAt,
      createdAt: notifications.createdAt,
    })
    .from(notifications)
    .where(eq(notifications.userId, userId))
    .orderBy(desc(notifications.createdAt))
    .limit(50);

  // Mark unread as read on view (fire-and-forget; UI shows the unread state
  // already at this point).
  await db
    .update(notifications)
    .set({ readAt: new Date() })
    .where(sql`${notifications.userId} = ${userId} AND ${notifications.readAt} IS NULL`);

  return (
    <div className="proto-page-aside">
      <AccountSidebar
        current="notifications"
        username={username ?? ""}
        asParam={sp.as}
      />
      <div className="proto-page-aside-content">
        <h1>Notifications</h1>
        <p className="proto-dek">@{username}&rsquo;s inbox.</p>
        <ul className="proto-notifications">
          {notes.length === 0 ? (
            <li className="proto-empty">No notifications.</li>
          ) : (
            notes.map((n) => (
              <li
                key={n.id}
                className={`proto-notification ${n.readAt ? "" : "unread"}`}
              >
                <Link href={buildLink(n.payload)} className="proto-notification-link">
                  <span className="proto-notification-kind">
                    {KIND_LABELS[n.kind] ?? n.kind}
                  </span>
                  <span className="proto-notification-body">
                    {(n.payload as NotePayload | null)?.commentId ? "New reply" : "New activity"}
                  </span>
                  <span className="proto-notification-time">
                    {relativeTime(n.createdAt.toISOString())}
                  </span>
                </Link>
              </li>
            ))
          )}
        </ul>
      </div>
    </div>
  );
}
