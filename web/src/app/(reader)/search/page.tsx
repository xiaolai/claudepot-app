import { sql } from "drizzle-orm";

import { auth } from "@/lib/auth";
import { db } from "@/db/client";
import { comments, submissions, users } from "@/db/schema";
import { ftsTsQuery } from "@/db/search-predicate";
import { getViewerVotesForSubmissions } from "@/db/queries";
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

/**
 * Reader-side FTS search. Sibling to lib/api/queries.ts:searchForApi:
 * both gate rows on `submissions.search_vec @@ websearch_to_tsquery
 * ('english', q)` so a future drop of the FTS column fails both
 * surfaces together. The two diverge on ordering (this page ranks by
 * ts_rank for browse-time relevance; the API orders by createdAt for
 * cursor-stable pagination) and on DTO shape (reader fixture-typed
 * here; SubmissionDto in the API). When editing the predicate here,
 * mirror it in searchForApi — the public-visibility filters
 * (state='approved', deletedAt IS NULL, unlistedAt IS NULL) must
 * stay in lockstep.
 */
async function search(q: string): Promise<Submission[]> {
  if (!q || q.trim().length < 2) return [];
  const tsQuery = ftsTsQuery(q);
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
      AND ${submissions.unlistedAt} IS NULL
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
  const session = await auth();
  const results = q ? await search(q) : [];
  const viewerVotes = session?.user?.id
    ? await getViewerVotesForSubmissions(
        session.user.id,
        results.map((s) => s.id),
      )
    : new Map<string, "up" | "down">();

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
