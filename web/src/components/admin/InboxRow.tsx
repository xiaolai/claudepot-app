import Link from "next/link";

import { ModButton } from "@/components/prototype/admin/ModButton";
import { relativeTime } from "@/lib/format";

/**
 * Discriminated union covering the four kinds of inbox items the
 * Today stream renders. Pending tags don't appear in the stream —
 * they live in the NoticeStrip — so they're absent here.
 *
 * Every row has the same skeleton: time + kind chip on the left,
 * subject + author + AI verdict in the middle, primary +
 * destructive button on the right. The CSS classes
 * `.proto-mod-btn-keep` (primary) and `.proto-mod-btn-remove`
 * (destructive) are also the keyboard-shortcut targets in
 * InboxKeyboard, so don't rename without updating that file too.
 */
export type InboxItem =
  | PendingSubmissionItem
  | OpenFlagItem
  | AppealItem;

export interface PendingSubmissionItem {
  kind: "pending_submission";
  id: string;
  createdAt: Date;
  title: string;
  url: string | null;
  type: string;
  authorUsername: string;
  authorKarma: number;
  authorPriorPosts: number;
  authorJoinedAt: Date;
}

export interface OpenFlagItem {
  kind: "open_flag";
  id: string;
  createdAt: Date;
  reporter: string;
  reason: string;
  targetType: "submission" | "comment" | "user";
  targetId: string;
  targetTitle: string | null;
  targetUserUsername: string | null;
}

export interface AppealItem {
  kind: "appeal";
  id: string;
  createdAt: Date;
  reporter: string;
  reasonBody: string;
  targetType: "submission" | "comment" | "user";
  targetId: string;
  targetTitle: string | null;
  decisionCategory: string | null;
  decisionConfidence: string | null;
  decisionOneLineWhy: string | null;
}

export function InboxRow({ item }: { item: InboxItem }) {
  switch (item.kind) {
    case "pending_submission":
      return <PendingSubmissionRow item={item} />;
    case "open_flag":
      return <OpenFlagRow item={item} />;
    case "appeal":
      return <AppealRow item={item} />;
  }
}

function PendingSubmissionRow({ item }: { item: PendingSubmissionItem }) {
  const authorContext =
    item.authorPriorPosts === 0
      ? `0 prior · joined ${relativeTime(item.authorJoinedAt.toISOString())}`
      : `${item.authorPriorPosts} prior · ${item.authorKarma} karma`;
  return (
    <article
      className="proto-inbox-row"
      data-inbox-row=""
      data-kind="pending_submission"
    >
      <span className="proto-inbox-row-meta">
        <span className="proto-inbox-row-time">
          {relativeTime(item.createdAt.toISOString())}
        </span>
        <span className="proto-inbox-row-kind">pending {item.type}</span>
      </span>
      <div className="proto-inbox-row-body">
        <div className="proto-inbox-row-title">
          {item.url ? (
            <a href={item.url} target="_blank" rel="noopener noreferrer">
              {item.title}
            </a>
          ) : (
            <Link href={`/post/${item.id}`}>{item.title}</Link>
          )}
        </div>
        <div className="proto-inbox-row-context">
          <Link href={`/u/${item.authorUsername}`}>
            @{item.authorUsername}
          </Link>{" "}
          · {authorContext}
        </div>
      </div>
      <div className="proto-inbox-row-actions">
        <ModButton
          action="approve"
          targetType="submission"
          targetId={item.id}
          className="proto-mod-btn proto-mod-btn-keep"
          pendingLabel="Approving…"
        >
          Approve
        </ModButton>
        <ModButton
          action="reject"
          targetType="submission"
          targetId={item.id}
          className="proto-mod-btn proto-mod-btn-remove"
          pendingLabel="Rejecting…"
        >
          Reject
        </ModButton>
      </div>
    </article>
  );
}

function OpenFlagRow({ item }: { item: OpenFlagItem }) {
  const isUserTarget = item.targetType === "user";
  const targetHref =
    item.targetType === "submission"
      ? `/post/${item.targetId}`
      : isUserTarget && item.targetUserUsername
        ? `/u/${item.targetUserUsername}`
        : "#";
  const targetLabel =
    item.targetTitle ??
    (isUserTarget && item.targetUserUsername
      ? `@${item.targetUserUsername}`
      : `${item.targetType} ${item.targetId.slice(0, 8)}…`);
  return (
    <article
      className="proto-inbox-row"
      data-inbox-row=""
      data-kind="open_flag"
    >
      <span className="proto-inbox-row-meta">
        <span className="proto-inbox-row-time">
          {relativeTime(item.createdAt.toISOString())}
        </span>
        <span className="proto-inbox-row-kind">flag</span>
      </span>
      <div className="proto-inbox-row-body">
        <div className="proto-inbox-row-title">
          <Link href={targetHref}>{targetLabel}</Link>
        </div>
        <div className="proto-inbox-row-context">
          @{item.reporter} reported · <em>{item.reason}</em>
        </div>
      </div>
      <div className="proto-inbox-row-actions">
        <ModButton
          action="dismiss_flag"
          targetId={item.targetId}
          flagId={item.id}
          className="proto-mod-btn proto-mod-btn-keep"
          pendingLabel="Dismissing…"
        >
          Dismiss
        </ModButton>
        {isUserTarget ? (
          <ModButton
            action="lock_user"
            targetId={item.targetId}
            flagId={item.id}
            className="proto-mod-btn proto-mod-btn-remove"
            pendingLabel="Suspending…"
          >
            Suspend user
          </ModButton>
        ) : (
          <ModButton
            action="delete"
            targetType={
              item.targetType === "comment" ? "comment" : "submission"
            }
            targetId={item.targetId}
            flagId={item.id}
            className="proto-mod-btn proto-mod-btn-remove"
            pendingLabel="Removing…"
          >
            Remove
          </ModButton>
        )}
      </div>
    </article>
  );
}

function AppealRow({ item }: { item: AppealItem }) {
  const targetHref =
    item.targetType === "submission"
      ? `/post/${item.targetId}`
      : item.targetType === "comment"
        ? `/post/${item.targetId}#comment`
        : "#";
  const decisionLine =
    item.decisionCategory && item.decisionConfidence
      ? `Ada: reject · ${item.decisionCategory} · conf=${item.decisionConfidence}`
      : "Ada decision unavailable";
  return (
    <article
      className="proto-inbox-row proto-inbox-row-appeal"
      data-inbox-row=""
      data-kind="appeal"
    >
      <span className="proto-inbox-row-meta">
        <span className="proto-inbox-row-time">
          {relativeTime(item.createdAt.toISOString())}
        </span>
        <span className="proto-inbox-row-kind">appeal</span>
      </span>
      <div className="proto-inbox-row-body">
        <div className="proto-inbox-row-title">
          <Link href={targetHref}>
            {item.targetTitle ??
              `${item.targetType} ${item.targetId.slice(0, 8)}…`}
          </Link>
        </div>
        <div className="proto-inbox-row-context">
          @{item.reporter} appealed · {decisionLine}
        </div>
        {item.decisionOneLineWhy ? (
          <div className="proto-inbox-row-why">
            <em>{item.decisionOneLineWhy}</em>
          </div>
        ) : null}
        {item.reasonBody ? (
          <div className="proto-inbox-row-reason">
            Their reason: {item.reasonBody}
          </div>
        ) : null}
      </div>
      <div className="proto-inbox-row-actions">
        <ModButton
          action="dismiss_flag"
          targetId={item.targetId}
          flagId={item.id}
          className="proto-mod-btn proto-mod-btn-keep"
          pendingLabel="Upholding…"
        >
          Uphold reject
        </ModButton>
        <ModButton
          action="restore"
          targetType={
            item.targetType === "comment" ? "comment" : "submission"
          }
          targetId={item.targetId}
          flagId={item.id}
          className="proto-mod-btn proto-mod-btn-remove"
          pendingLabel="Restoring…"
        >
          Restore
        </ModButton>
      </div>
    </article>
  );
}
