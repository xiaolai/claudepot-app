import { sql } from "drizzle-orm";

import { db } from "@/db/client";
import { comments, submissions, users } from "@/db/schema";
import { SubmissionRow } from "@/components/prototype/SubmissionRow";
import { deriveDomain } from "@/lib/url";

import type { Submission } from "@/lib/prototype-fixtures";

type SearchRow = {
  id: string;
  type: Submission["type"];
  title: string;
  url: string | null;
  text: string | null;
  state: Submission["state"];
  score: number;
  comments_count: number;
  created_at: string;
  author_username: string;
};

async function search(q: string): Promise<Submission[]> {
  if (!q || q.trim().length < 2) return [];
  const tsQuery = sql`websearch_to_tsquery('english', ${q})`;
  const rows = await db.execute<SearchRow>(sql`
    SELECT
      ${submissions.id} AS id,
      ${submissions.type} AS type,
      ${submissions.title} AS title,
      ${submissions.url} AS url,
      ${submissions.text} AS text,
      ${submissions.state} AS state,
      ${submissions.score} AS score,
      (
        SELECT COUNT(*)::int FROM ${comments}
        WHERE ${comments.submissionId} = ${submissions.id}
          AND ${comments.deletedAt} IS NULL
      ) AS comments_count,
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

  const list = (rows.rows ?? []) as SearchRow[];
  return list.map((r) => ({
    id: r.id,
    user: r.author_username,
    type: r.type,
    tags: [],
    title: r.title,
    url: r.url ?? null,
    domain: deriveDomain(r.url ?? null) ?? "",
    subjects: [],
    upvotes: Math.max(Number(r.score) ?? 0, 0),
    downvotes: Math.max(-(Number(r.score) ?? 0), 0),
    comments: Number(r.comments_count) || 0,
    submitted_at: new Date(r.created_at).toISOString(),
    text: r.text ?? undefined,
    state: r.state,
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
