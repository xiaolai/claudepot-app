import { sql } from "drizzle-orm";

import { db } from "@/db/client";
import { submissions, users } from "@/db/schema";
import { SubmissionRow } from "@/components/prototype/SubmissionRow";

import type { Submission } from "@/lib/prototype-fixtures";

function deriveDomain(url: string | null): string {
  if (!url) return "sha.com";
  try {
    return new URL(url).hostname;
  } catch {
    return "";
  }
}

async function search(q: string): Promise<Submission[]> {
  if (!q || q.trim().length < 2) return [];
  const tsQuery = sql`websearch_to_tsquery('english', ${q})`;
  const rows = await db.execute(sql`
    SELECT
      ${submissions.id} AS id,
      ${submissions.type} AS type,
      ${submissions.title} AS title,
      ${submissions.url} AS url,
      ${submissions.text} AS text,
      ${submissions.state} AS state,
      ${submissions.score} AS score,
      ${submissions.createdAt} AS created_at,
      ${users.username} AS author_username,
      ts_rank(${submissions}.search_vec, ${tsQuery}) AS rank
    FROM ${submissions}
    INNER JOIN ${users} ON ${users.id} = ${submissions.authorId}
    WHERE ${submissions.state} = 'approved'
      AND ${submissions.deletedAt} IS NULL
      AND ${submissions}.search_vec @@ ${tsQuery}
    ORDER BY rank DESC
    LIMIT 30
  `);

  // drizzle-orm's db.execute returns a result object; rows is typically `.rows`.
  // Cast through unknown for the dynamic shape.
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const list = (rows as any).rows ?? rows;
  return (list as Array<Record<string, unknown>>).map((r) => ({
    id: r.id as string,
    user: r.author_username as string,
    type: r.type as Submission["type"],
    tags: [],
    title: r.title as string,
    url: (r.url as string) ?? null,
    domain: deriveDomain((r.url as string) ?? null),
    subjects: [],
    upvotes: Math.max(Number(r.score) ?? 0, 0),
    downvotes: Math.max(-(Number(r.score) ?? 0), 0),
    comments: 0,
    submitted_at: new Date(r.created_at as string).toISOString(),
    text: (r.text as string) ?? undefined,
    state: r.state as Submission["state"],
  }));
}

export default async function SearchPage({
  searchParams,
}: {
  searchParams: Promise<{ q?: string }>;
}) {
  const sp = await searchParams;
  const q = sp.q?.trim() ?? "";
  const results = q ? await search(q) : [];

  return (
    <div className="proto-page">
      <h1>Search</h1>
      <form className="proto-form proto-form-inline" method="GET">
        <input
          type="search"
          name="q"
          placeholder="Search titles, text, URLs…"
          defaultValue={q}
          className="proto-input proto-input-wide"
          autoFocus
        />
        <button type="submit" className="proto-btn-primary">
          Search
        </button>
      </form>

      {q && (
        <p className="proto-dek">
          {results.length} result{results.length === 1 ? "" : "s"} for{" "}
          <strong>{q}</strong>.
        </p>
      )}

      <ol className="proto-feed">
        {results.length === 0 && q ? (
          <li className="proto-empty">No matches.</li>
        ) : (
          results.map((s, i) => (
            <SubmissionRow key={s.id} rank={i + 1} submission={s} />
          ))
        )}
      </ol>
    </div>
  );
}
