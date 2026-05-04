import { FeedHeader } from "@/components/prototype/FeedHeader";
import { SubmissionRow } from "@/components/prototype/SubmissionRow";
import { auth } from "@/lib/auth";
import { getSubmissionsByNew } from "@/db/queries";

export default async function NewFeed() {
  const session = await auth();
  const items = await getSubmissionsByNew(session?.user?.id ?? null, 30);

  return (
    <div className="proto-page">
      <FeedHeader />
      <ol className="proto-feed">
        {items.map((s, i) => (
          <SubmissionRow key={s.id} rank={i + 1} submission={s} />
        ))}
      </ol>
    </div>
  );
}
