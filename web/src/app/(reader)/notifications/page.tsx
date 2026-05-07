import type { ReactNode } from "react";
import Link from "next/link";
import { and, desc, eq, inArray, isNull } from "drizzle-orm";
import { AtSign, Check, CornerDownRight } from "lucide-react";

import { db } from "@/db/client";
import { comments, notifications, submissions, users } from "@/db/schema";
import { auth } from "@/lib/auth";
import { relativeTime } from "@/lib/format";
import { getCurrentUser } from "@/lib/auth-shim";
import { markAllReadForUser } from "@/lib/notifications";
import { AccountSidebar } from "@/components/prototype/AccountSidebar";

const NOTIFICATION_BODY_PREVIEW_CHARS = 140;

function previewBody(body: string): string {
  const trimmed = body.trim();
  if (trimmed.length <= NOTIFICATION_BODY_PREVIEW_CHARS) return trimmed;
  return trimmed.slice(0, NOTIFICATION_BODY_PREVIEW_CHARS).trimEnd() + "…";
}

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
  // Moderation notifications carry a different shape — see
  // lib/moderation/notify.ts. The two payload variants coexist here
  // because notifications.payload is `jsonb` and per-kind dispatch
  // happens at render time.
  //
  // appeal_url can be explicitly null on illegal-comment blocks (no
  // appealable target exists). decision_id is still set, but we MUST
  // NOT reconstruct an /appeal/[id] link from it — the appeal core
  // returns "stale" because target_id was never set, so a deep-link
  // would dead-end the user.
  appeal_url?: string | null;
  decision_id?: string;
};

function buildLink(payload: unknown): string {
  const p = (payload ?? {}) as NotePayload;
  // Honor an explicit null appeal_url — the notification's author
  // intentionally suppressed the appeal CTA.
  if (p.appeal_url === null) return "#";
  if (p.appeal_url) return p.appeal_url;
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

  // Enrich reply/mention rows with the commenting author's username and
  // a body excerpt so the inbox surfaces "@actor: snippet…" instead of
  // a generic "New reply." Moderation rows carry their own copy via the
  // payload (decision_id / appeal_url) and don't need the lookup.
  const commentIds = Array.from(
    new Set(
      notes
        .map((n) => (n.payload as NotePayload | null)?.commentId)
        .filter((id): id is string => typeof id === "string"),
    ),
  );

  type CommentEnrichment = { author: string; body: string };
  const commentMap = new Map<string, CommentEnrichment>();
  if (commentIds.length > 0) {
    // Apply the same visibility filters the public thread renders
    // with: only approved, non-deleted comments whose parent
    // submission is itself approved and visible. Without these,
    // a moderator-rejected reply or a soft-deleted comment would
    // still leak its body into the recipient's notification snippet
    // — even though the click-through would tombstone it.
    const rows = await db
      .select({
        id: comments.id,
        body: comments.body,
        author: users.username,
      })
      .from(comments)
      .innerJoin(users, eq(users.id, comments.authorId))
      .innerJoin(submissions, eq(submissions.id, comments.submissionId))
      .where(
        and(
          inArray(comments.id, commentIds),
          eq(comments.state, "approved"),
          isNull(comments.deletedAt),
          eq(submissions.state, "approved"),
          isNull(submissions.deletedAt),
          isNull(submissions.unlistedAt),
        ),
      );
    for (const r of rows) {
      commentMap.set(r.id, { author: r.author, body: r.body });
    }
  }

  // Mark unread as read on view. The UI snapshotted readAt above so
  // unread items still render with the unread style on this render.
  // Shares lib/notifications.markAllReadForUser with the API surface
  // so both consume notifications the same way.
  await markAllReadForUser(userId);

  function bodyFor(n: (typeof notes)[number]): ReactNode {
    const p = (n.payload ?? {}) as NotePayload;
    if (p.commentId) {
      const enrich = commentMap.get(p.commentId);
      if (enrich) {
        return (
          <>
            <span className="proto-notification-actor">@{enrich.author}</span>
            <span className="proto-notification-snippet">
              {previewBody(enrich.body)}
            </span>
          </>
        );
      }
      return "New reply";
    }
    if (n.kind === "moderation") {
      return p.appeal_url
        ? "Your submission was rejected — appeal available."
        : "Moderation decision on your content.";
    }
    return "New activity";
  }

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
                  <span className="proto-notification-body">{bodyFor(n)}</span>
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
