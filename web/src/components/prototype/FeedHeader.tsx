import Link from "next/link";
import { FeedTabs } from "./FeedTabs";
import { getTopTags } from "@/db/queries";

const TAG_PILLS = 8;

export async function FeedHeader() {
  const topTags = (await getTopTags()).slice(0, TAG_PILLS);

  return (
    <>
      <h1>The feed</h1>
      <p className="proto-dek">
        A daily reader for builders working with AI tools.
      </p>

      <nav className="proto-tag-pills" aria-label="Filter by tag">
        {topTags.map((t) => (
          <Link key={t.slug} href={`/c/${t.slug}`} className="proto-tag-pill">
            <span className="proto-tag-pill-name">{t.name}</span>
            {t.count > 0 && (
              <span className="proto-tag-pill-count">{t.count}</span>
            )}
          </Link>
        ))}
        <Link href="/c" className="proto-tag-pill proto-tag-pill-all">
          All tags →
        </Link>
      </nav>

      <FeedTabs />
    </>
  );
}
