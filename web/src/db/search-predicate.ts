/**
 * Shared FTS predicate for submission search.
 *
 * Both the reader's /search page and the v1 API's searchForApi gate
 * rows on the same Postgres FTS expression:
 *
 *   submissions.search_vec @@ websearch_to_tsquery('english', q)
 *
 * Keeping the predicate here rather than copying the SQL into both
 * call sites means a future change to the FTS column or the
 * tokenizer flows through one edit, not two. Per the
 * .claude/rules/db-migrations.md push-drop hazard, both surfaces
 * also break together if `search_vec` is ever dropped — that's a
 * deliberate single-blast-radius design.
 *
 * The two callers still differ in ordering (reader: ts_rank for
 * relevance; API: createdAt for cursor stability) and DTO shape;
 * those concerns stay in their respective files.
 */

import { sql } from "drizzle-orm";

import { submissions } from "./schema";

/** Build the `websearch_to_tsquery` for a user query. Postgres handles
 *  malformed input by returning an empty tsquery (no matches), so the
 *  caller doesn't need to pre-validate q for ts_query syntax. */
export function ftsTsQuery(q: string) {
  return sql`websearch_to_tsquery('english', ${q})`;
}

/** Full FTS gate predicate `submissions.search_vec @@ <tsQuery>`. */
export function ftsSubmissionMatch(q: string) {
  return sql`${submissions}.search_vec @@ ${ftsTsQuery(q)}`;
}
