import Link from "next/link";
import { FeedHeader } from "@/components/prototype/FeedHeader";
import { SubmissionRow } from "@/components/prototype/SubmissionRow";
import { auth } from "@/lib/auth";
import { getSubmissionsByTop } from "@/db/queries";

const RANGES = [
  { key: "day",  label: "Today" },
  { key: "week", label: "This week" },
  { key: "all",  label: "All time" },
] as const;

export default async function TopFeed({
  searchParams,
}: {
  searchParams: Promise<{ range?: string }>;
}) {
  const params = await searchParams;
  const range =
    params.range === "week" || params.range === "all" ? params.range : "day";
  const session = await auth();
  const items = await getSubmissionsByTop(
    range,
    session?.user?.id ?? null,
    30,
  );

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
          <li className="proto-empty">No submissions in this range.</li>
        ) : (
          items.map((s, i) => (
            <SubmissionRow key={s.id} rank={i + 1} submission={s} />
          ))
        )}
      </ol>
    </div>
  );
}
