import Link from "next/link";
import { getTopTags } from "@/db/queries";
import { randomCardTint } from "@/lib/card-tint";

export default async function TagsIndex() {
  const tags = await getTopTags();

  return (
    <div className="proto-page">
      <h1>Tags</h1>
      <p className="proto-dek">
        Tags are flat and multi-valued. AI assigns them automatically when a
        post is approved; staff curate the vocabulary.
      </p>
      <div className="proto-projects-grid">
        {tags.map((t) => {
          const tint = randomCardTint();
          return (
            <Link
              key={t.slug}
              href={`/c/${t.slug}`}
              className="proto-project-card proto-project-card-tinted"
              style={{ ["--card-tint" as string]: tint }}
            >
              <h2 className="proto-project-card-name">{t.name}</h2>
              <p className="proto-project-card-tagline">{t.tagline}</p>
              <div className="proto-project-card-meta">
                <span>{t.count} this week</span>
              </div>
            </Link>
          );
        })}
      </div>
    </div>
  );
}
