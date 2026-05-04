import Link from "next/link";
import type { Submission } from "@/lib/prototype-fixtures";
import { TYPE_LABELS, relativeTime } from "@/lib/format";
import { UserAvatar } from "./Avatar";

interface Props {
  submission: Submission;
  /** Whether to render the comment count. Hide it on pending/rejected items. */
  showCommentCount: boolean;
  /** Override the cached count — post-detail uses a recursive tree count. */
  commentCount?: number;
  /** Whether the comment count is a link to /post/[id]. Default true. */
  linkComments?: boolean;
  /** Tag override — used when showing AI-proposed tags for pending items. */
  tags?: string[];
}

/**
 * The byline-row shared by SubmissionRow (feed) and the post-detail
 * page. Type label, tags, author with avatar, relative time, and
 * optional comment count — same structure in both surfaces.
 */
export function SubmissionMeta({
  submission: s,
  showCommentCount,
  commentCount,
  linkComments = true,
  tags,
}: Props) {
  const displayTags = tags ?? s.tags;
  const count = commentCount ?? s.comments;
  return (
    <div className="proto-row-meta">
      <span className="proto-type">{TYPE_LABELS[s.type]}</span>
      {displayTags.length > 0 && (
        <span className="proto-subjects">
          {displayTags.map((t) => (
            <Link key={t} href={`/c/${t}`} className="proto-subject">
              {t}
            </Link>
          ))}
        </span>
      )}
      <span className="sep">·</span>
      <span>
        by{" "}
        <Link href={`/u/${s.user}`} className="proto-row-author-link">
          <UserAvatar
            username={s.user}
            imageUrl={s.user_image_url}
            size={16}
          />
          <span>
            {s.user}
            {s.auto_posted ? " 🤖" : ""}
          </span>
        </Link>
      </span>
      <span className="sep">·</span>
      <span>{relativeTime(s.submitted_at)}</span>
      {showCommentCount && (
        <>
          <span className="sep">·</span>
          {linkComments ? (
            <Link href={`/post/${s.id}`}>{count} comments</Link>
          ) : (
            <span>{count} comments</span>
          )}
        </>
      )}
    </div>
  );
}
