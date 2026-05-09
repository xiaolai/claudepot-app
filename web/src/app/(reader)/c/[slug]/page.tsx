import Link from "next/link";
import { notFound } from "next/navigation";
import { SubmissionRow } from "@/components/prototype/SubmissionRow";
import { FollowTagButton } from "@/components/prototype/FollowTagButton";
import { auth } from "@/lib/auth";
import {
  getAllTags,
  getSubmissionsByTag,
  getTagBySlug,
  getViewerVotesForSubmissions,
  isTagFollowed,
} from "@/db/queries";
import { decodeCursor, isCursorTime } from "@/lib/api/cursor";

/**
 * Force dynamic rendering. The page reads `auth()` (cookies), so it's
 * implicitly dynamic at request time — but on 2026-05-06 a route audit
 * found that unknown slugs returned 500 (Pages Router `/_error` shell)
 * instead of 404 (App Router not-found.tsx). The 500 was triggered by
 * the combination of `generateStaticParams` returning [] (empty tags
 * table) plus Next.js's default static-optimization heuristic, which
 * marked the route as fully-prerendered with no params.
 *
 * `force-dynamic` preserves `generateStaticParams` as a build-time hint
 * once tags exist (the prerender will still seed known slugs), but
 * guarantees unknown slugs go through dynamic render → notFound() →
 * 404 every time.
 */
export const dynamic = "force-dynamic";

export async function generateStaticParams() {
  return (await getAllTags()).map((t) => ({ slug: t.slug }));
}

export default async function TagPage({
  params,
  searchParams,
}: {
  params: Promise<{ slug: string }>;
  searchParams: Promise<{ cursor?: string }>;
}) {
  const { slug } = await params;
  const sp = await searchParams;
  const tag = await getTagBySlug(slug);
  if (!tag) notFound();

  const session = await auth();
  const viewerId = session?.user?.id ?? null;
  const decoded = decodeCursor(sp.cursor);
  const cursor = decoded && isCursorTime(decoded) ? decoded : null;
  const [page, followed] = await Promise.all([
    getSubmissionsByTag(slug, { viewerId, cursor }),
    isTagFollowed(viewerId, slug),
  ]);
  const { items, nextCursor } = page;
  const viewerVotes = viewerId
    ? await getViewerVotesForSubmissions(viewerId, items.map((s) => s.id))
    : new Map<string, "up" | "down">();

  return (
    <div className="proto-page">
      <header className="proto-tag-header">
        <p className="eyebrow">Tag</p>
        <h1>{tag.name}</h1>
        <p className="proto-dek">{tag.tagline}</p>
        <p className="proto-tag-meta">
          <Link href="/c">All tags</Link> ·{" "}
          <Link href={`/api/rss/c/${slug}`}>RSS</Link>
        </p>
        <FollowTagButton
          tagSlug={slug}
          initialFollowed={followed}
          signedIn={Boolean(viewerId)}
        />
      </header>

      <ol className="proto-feed">
        {items.length === 0 ? (
          <li className="proto-empty">Nothing tagged {tag.name} yet.</li>
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
          <Link href={`/c/${slug}?cursor=${encodeURIComponent(nextCursor)}`}>
            Older →
          </Link>
        </p>
      ) : null}
    </div>
  );
}
