import { notFound } from "next/navigation";
import Link from "next/link";
import { SubmissionRow } from "@/components/prototype/SubmissionRow";
import { UserAvatar } from "@/components/prototype/Avatar";
import {
  getCommentsByUser,
  getSubmissionsByUser,
  getUser,
} from "@/db/queries";
import { relativeTime } from "@/lib/format";
import { decodeCursor, isCursorTime } from "@/lib/api/cursor";

const TABS = ["submissions", "comments"] as const;
type Tab = (typeof TABS)[number];

const TAB_LABELS: Record<Tab, string> = {
  submissions: "Submissions",
  comments: "Comments",
};

const COMMENT_PREVIEW_CHARS = 280;

function previewBody(body: string): string {
  const trimmed = body.trim();
  if (trimmed.length <= COMMENT_PREVIEW_CHARS) return trimmed;
  return trimmed.slice(0, COMMENT_PREVIEW_CHARS).trimEnd() + "…";
}

export default async function ProfilePage({
  params,
  searchParams,
}: {
  params: Promise<{ username: string }>;
  searchParams: Promise<{ tab?: string; as?: string; cursor?: string }>;
}) {
  const { username } = await params;
  const sp = await searchParams;
  const tab: Tab = TABS.includes(sp.tab as Tab) ? (sp.tab as Tab) : "submissions";

  const user = await getUser(username);
  if (!user) notFound();

  const decoded = decodeCursor(sp.cursor);
  const cursor = decoded && isCursorTime(decoded) ? decoded : null;

  const submissionsPage =
    tab === "submissions"
      ? await getSubmissionsByUser(username, { cursor })
      : { items: [], nextCursor: null as string | null };
  const commentsPage =
    tab === "comments"
      ? await getCommentsByUser(username, { cursor })
      : { items: [], nextCursor: null as string | null };

  const linkSuffix = sp.as ? `&as=${sp.as}` : "";
  const baseSuffix = sp.as ? `?as=${sp.as}` : "";

  const activeNextCursor =
    tab === "submissions" ? submissionsPage.nextCursor : commentsPage.nextCursor;
  const olderHref =
    activeNextCursor &&
    `/u/${username}?${tab !== "submissions" ? `tab=${tab}&` : ""}cursor=${encodeURIComponent(activeNextCursor)}${linkSuffix}`;

  return (
    <div className="proto-page">
      <header className="proto-profile-header">
        <UserAvatar
          username={user.username}
          imageUrl={user.image_url}
          size={64}
        />
        <div className="proto-profile-header-text">
          <h1>{user.display_name}</h1>
          <span className="proto-profile-meta">@{user.username}</span>
        </div>
      </header>
      <p className="proto-profile-bio">{user.bio}</p>
      <div className="proto-profile-meta">
        karma {user.karma} · joined {user.joined} · via {user.provider}
      </div>

      <nav className="proto-tabs" aria-label="Profile sections">
        {TABS.map((t) => (
          <Link
            key={t}
            href={
              t === "submissions"
                ? `/u/${username}${baseSuffix}`
                : `/u/${username}?tab=${t}${linkSuffix}`
            }
            aria-current={tab === t ? "page" : undefined}
          >
            {TAB_LABELS[t]}
          </Link>
        ))}
      </nav>

      <ol className="proto-feed">
        {tab === "submissions" &&
          (submissionsPage.items.length === 0 ? (
            <li className="proto-empty">No submissions yet.</li>
          ) : (
            submissionsPage.items.map((s) => (
              <SubmissionRow key={s.id} submission={s} />
            ))
          ))}
        {tab === "comments" &&
          (commentsPage.items.length === 0 ? (
            <li className="proto-empty">No comments yet.</li>
          ) : (
            commentsPage.items.map((c) => (
              <li key={c.id} className="proto-profile-comment">
                <p className="proto-profile-comment-meta">
                  on{" "}
                  <Link href={`/post/${c.submissionId}#comment-${c.id}`}>
                    {c.submissionTitle}
                  </Link>{" "}
                  · {relativeTime(c.submitted_at)} · {c.score} point
                  {c.score === 1 ? "" : "s"}
                </p>
                <p className="proto-profile-comment-body">
                  {previewBody(c.body)}
                </p>
              </li>
            ))
          ))}
      </ol>
      {olderHref ? (
        <p className="proto-pagination">
          <Link href={olderHref}>Older →</Link>
        </p>
      ) : null}
    </div>
  );
}
