import Link from "next/link";
import { FeedHeader } from "@/components/prototype/FeedHeader";
import { SubmissionRow } from "@/components/prototype/SubmissionRow";
import { EmptyFeedState } from "@/components/prototype/EmptyFeedState";
import { auth } from "@/lib/auth";
import { getSubmissionsByTop, getViewerVotesForSubmissions } from "@/db/queries";

const RANGES = [
  { key: "day",  label: "Today" },
  { key: "week", label: "This week" },
  { key: "all",  label: "All time" },
] as const;

/**
 * /top is a single-page leaderboard — no cursor pagination. The audit
 * round-2 finding flagged that a (score, id) cursor on a mutable
 * score column allows skip/duplicate at page boundaries when votes
 * shift a row's score across the cursor between page reads. Users
 * who want depth go to /new (immutable createdAt cursor) or filter
 * by tag.
 */
export default async function TopFeed({
  searchParams,
}: {
  searchParams: Promise<{ range?: string }>;
}) {
  const params = await searchParams;
  const range =
    params.range === "week" || params.range === "all" ? params.range : "day";
  const session = await auth();
  const items = await getSubmissionsByTop({
    range,
    viewerId: session?.user?.id ?? null,
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
      <nav className="proto-tabs" aria-label="Top range">
        {RANGES.map((r) => (
          <Link
            key={r.key}
            href={r.key === "day" ? "/top" : `/top?range=${r.key}`}
            aria-current={r.key === range ? "page" : undefined}
          >
            {r.label}
          </Link>
        ))}
      </nav>
      <ol className="proto-feed">
        {items.length === 0 ? (
          <EmptyFeedState message={`No top posts in ${range === "day" ? "the last 24 hours" : range === "week" ? "the last week" : "all time"}.`} />
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
    </div>
  );
}
