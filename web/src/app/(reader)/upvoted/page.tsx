import Link from "next/link";
import { ChevronUp } from "lucide-react";
import { PersonalHubFeed } from "@/components/prototype/PersonalHubFeed";
import { getUpvotedByUser } from "@/db/queries";

export default async function UpvotedInbox({
  searchParams,
}: {
  searchParams: Promise<{ as?: string }>;
}) {
  const sp = await searchParams;
  return (
    <PersonalHubFeed
      searchParams={sp}
      current="upvoted"
      title="Upvoted"
      signedOutDek={
        <>
          Posts you&rsquo;ve upvoted. <Link href="/login">Sign in</Link> to
          see them.
        </>
      }
      dek={
        <>
          Tap the{" "}
          <span className="proto-inline-icon" aria-label="upvote">
            <ChevronUp size={14} aria-hidden />
          </span>{" "}
          on any post to add it here.
        </>
      }
      emptyText="Nothing upvoted yet."
      loader={getUpvotedByUser}
      rowMarkers={{ initialVote: "up" }}
    />
  );
}
