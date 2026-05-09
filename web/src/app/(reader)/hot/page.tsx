import { SubmissionRow } from "@/components/prototype/SubmissionRow";
import { FeedHeader } from "@/components/prototype/FeedHeader";
import { EmptyFeedState } from "@/components/prototype/EmptyFeedState";
import { auth } from "@/lib/auth";
import { getSubmissionsByHot, getViewerVotesForSubmissions } from "@/db/queries";

const FEED_LIMIT = 25;

export default async function HotFeed() {
  const session = await auth();
  const feed = await getSubmissionsByHot(
    session?.user?.id ?? null,
    FEED_LIMIT,
  );
  const viewerVotes = session?.user?.id
    ? await getViewerVotesForSubmissions(
        session.user.id,
        feed.map((s) => s.id),
      )
    : new Map<string, "up" | "down">();

  return (
    <div className="proto-page">
      <FeedHeader />

      <ol className="proto-feed">
        {feed.length === 0 ? (
          <EmptyFeedState message="No posts yet — be the first." />
        ) : (
          feed.map((s, i) => (
            <SubmissionRow
              key={s.id}
              rank={i + 1}
              submission={s}
              initialVote={viewerVotes.get(s.id) ?? null}
            />
          ))
        )}
      </ol>
    </div>
  );
}
