import Link from "next/link";

import { SubmissionRow } from "@/components/prototype/SubmissionRow";
import { FeedHeader } from "@/components/prototype/FeedHeader";
import { EmptyFeedState } from "@/components/prototype/EmptyFeedState";
import { auth } from "@/lib/auth";
import { getSubmissionsByNew, getViewerVotesForSubmissions } from "@/db/queries";
import { decodeCursor, isCursorTime } from "@/lib/api/cursor";

export default async function Home({
  searchParams,
}: {
  searchParams: Promise<{ cursor?: string }>;
}) {
  const sp = await searchParams;
  const session = await auth();
  const decoded = decodeCursor(sp.cursor);
  const cursor = decoded && isCursorTime(decoded) ? decoded : null;
  const { items, nextCursor } = await getSubmissionsByNew({
    viewerId: session?.user?.id ?? null,
    cursor,
  });
  const viewerVotes = session?.user?.id
    ? await getViewerVotesForSubmissions(
        session.user.id,
        items.map((s) => s.id),
      )
    : new Map<string, "up" | "down">();

  return (
    <div className="proto-page">
      <FeedHeader />

      <ol className="proto-feed">
        {items.length === 0 ? (
          <EmptyFeedState message="Nothing new yet." />
        ) : (
          items.map((s, i) => (
            <SubmissionRow
              key={s.id}
              rank={i + 1}
              submission={s}
              initialVote={viewerVotes.get(s.id) ?? null}
            />
          ))
        )}
      </ol>
      {nextCursor ? (
        <p className="proto-pagination">
          <Link href={`/?cursor=${encodeURIComponent(nextCursor)}`}>
            Older →
          </Link>
        </p>
      ) : null}
    </div>
  );
}
