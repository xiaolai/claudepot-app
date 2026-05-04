import Link from "next/link";
import { notFound } from "next/navigation";
import { SubmissionRow } from "@/components/prototype/SubmissionRow";
import { auth } from "@/lib/auth";
import { getAllTags, getTagBySlug, getSubmissionsByTag } from "@/db/queries";

export async function generateStaticParams() {
  return (await getAllTags()).map((t) => ({ slug: t.slug }));
}

export default async function TagPage({
  params,
}: {
  params: Promise<{ slug: string }>;
}) {
  const { slug } = await params;
  const tag = await getTagBySlug(slug);
  if (!tag) notFound();

  const session = await auth();
  const items = await getSubmissionsByTag(slug, session?.user?.id ?? null, 60);

  return (
    <div className="proto-page">
      <header className="proto-tag-header">
        <p className="eyebrow">Tag</p>
        <h1>{tag.name}</h1>
        <p className="proto-dek">{tag.tagline}</p>
        <p className="proto-tag-meta">
          {items.length} {items.length === 1 ? "post" : "posts"} ·{" "}
          <Link href="/c">All tags</Link>
        </p>
      </header>

      <ol className="proto-feed">
        {items.length === 0 ? (
          <li className="proto-empty">Nothing tagged {tag.name} yet.</li>
        ) : (
          items.map((s, i) => (
            <SubmissionRow key={s.id} rank={i + 1} submission={s} />
          ))
        )}
      </ol>
    </div>
  );
}
