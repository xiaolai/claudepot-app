import Link from "next/link";
import { FeedHeader } from "@/components/prototype/FeedHeader";
import { SubmissionRow } from "@/components/prototype/SubmissionRow";
import { EmptyFeedState } from "@/components/prototype/EmptyFeedState";
import { auth } from "@/lib/auth";
import { getSubmissionsByTop } from "@/db/queries";
import { decodeCursor, isCursorScore } from "@/lib/api/cursor";

const RANGES = [
  { key: "day",  label: "Today" },
  { key: "week", label: "This week" },
  { key: "all",  label: "All time" },
] as const;

export default async function TopFeed({
  searchParams,
}: {
  searchParams: Promise<{ range?: string; cursor?: string }>;
}) {
  const params = await searchParams;
  const range =
    params.range === "week" || params.range === "all" ? params.range : "day";
  const session = await auth();
  const decoded = decodeCursor(params.cursor);
  const cursor = decoded && isCursorScore(decoded) ? decoded : null;
  const { items, nextCursor } = await getSubmissionsByTop({
    range,
    viewerId: session?.user?.id ?? null,
    cursor,
  });

  const olderHref =
    nextCursor &&
    `/top?${range !== "day" ? `range=${range}&` : ""}cursor=${encodeURIComponent(nextCursor)}`;

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
            <SubmissionRow key={s.id} rank={i + 1} submission={s} />
          ))
        )}
      </ol>
      {olderHref ? (
        <p className="proto-pagination">
          <Link href={olderHref}>Older →</Link>
        </p>
      ) : null}
    </div>
  );
}
