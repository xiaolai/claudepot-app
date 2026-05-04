import { SubmissionRow } from "@/components/prototype/SubmissionRow";
import { FeedHeader } from "@/components/prototype/FeedHeader";
import { auth } from "@/lib/auth";
import { getSubmissionsByHot } from "@/db/queries";

const FEED_LIMIT = 25;

export default async function Home() {
  const session = await auth();
  const feed = await getSubmissionsByHot(
    session?.user?.id ?? null,
    FEED_LIMIT,
  );

  return (
    <div className="proto-page">
      <FeedHeader />

      <ol className="proto-feed">
        {feed.map((s, i) => (
          <SubmissionRow key={s.id} rank={i + 1} submission={s} />
        ))}
      </ol>
    </div>
  );
}
