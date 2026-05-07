import { eq, sql } from "drizzle-orm";

import { db } from "@/db/client";
import { submissions, submissionTags, tags } from "@/db/schema";
import { CreateTagForm } from "@/components/prototype/admin/CreateTagForm";
import { PendingTagRow } from "@/components/prototype/admin/PendingTagRow";
import { TagRow } from "@/components/prototype/admin/TagRow";
import { staffGate } from "@/lib/staff-gate";

const PENDING_SAMPLE_LIMIT = 3;

export default async function AdminFlags({
  searchParams,
}: {
  searchParams: Promise<{ as?: string }>;
}) {
  const sp = await searchParams;
  const gate = await staffGate(sp);
  if (gate) return gate;

  // Group counts in one round-trip instead of N+1 queries. Filter
  // out pending_review=true rows — those render in the dedicated
  // section above with their own approve/reject actions.
  const rows = await db
    .select({
      slug: tags.slug,
      name: tags.name,
      tagline: tags.tagline,
      sortOrder: tags.sortOrder,
      posts: sql<number>`(
        SELECT COUNT(*)::int FROM ${submissionTags}
        WHERE ${submissionTags.tagSlug} = ${tags.slug}
      )`,
    })
    .from(tags)
    .where(eq(tags.pendingReview, false))
    .orderBy(tags.sortOrder);

  // Migration 0022 — pending Ada-proposed tags awaiting staff review.
  // We pull post counts inline so staff sees how many submissions
  // would lose this tag if they reject it. Sample titles below.
  const pendingRows = await db
    .select({
      slug: tags.slug,
      name: tags.name,
      tagline: tags.tagline,
      postCount: sql<number>`(
        SELECT COUNT(*)::int FROM ${submissionTags}
        WHERE ${submissionTags.tagSlug} = ${tags.slug}
      )`,
    })
    .from(tags)
    .where(eq(tags.pendingReview, true))
    .orderBy(tags.slug);

  // Fetch sample titles for every pending tag in ONE query using
  // ROW_NUMBER() OVER (PARTITION BY tag_slug ORDER BY created_at DESC).
  // The CTE picks the top N per tag, then we group in JS. Avoids
  // the previous N+1 (one query per pending tag) so the page stays
  // responsive even if a quiet vocabulary suddenly accumulates a
  // long pending list (e.g. after a model upgrade that changes
  // tagging behavior).
  const samplesByTag: Record<string, string[]> = {};
  const pendingSlugs = pendingRows.map((t) => t.slug);
  if (pendingSlugs.length > 0) {
    // sql.join builds the literal `($1, $2, ...)` IN clause with
    // proper parameter binding — passing an array directly to sql``
    // would interpolate as text and break.
    const slugList = sql.join(
      pendingSlugs.map((s) => sql`${s}`),
      sql`, `,
    );
    const sampleRows = await db.execute<{
      tag_slug: string;
      title: string;
    }>(sql`
      SELECT tag_slug, title FROM (
        SELECT
          ${submissionTags.tagSlug} AS tag_slug,
          ${submissions.title} AS title,
          ROW_NUMBER() OVER (
            PARTITION BY ${submissionTags.tagSlug}
            ORDER BY ${submissions.createdAt} DESC
          ) AS rn
        FROM ${submissionTags}
        INNER JOIN ${submissions}
          ON ${submissions.id} = ${submissionTags.submissionId}
        WHERE ${submissionTags.tagSlug} IN (${slugList})
      ) ranked
      WHERE rn <= ${PENDING_SAMPLE_LIMIT}
      ORDER BY tag_slug, rn
    `);
    // db.execute returns either { rows: [...] } (pg adapter) or
    // an array directly (neon-http) — handle both shapes.
    const rows: Array<{ tag_slug: string; title: string }> = Array.isArray(
      sampleRows,
    )
      ? sampleRows
      : (sampleRows as { rows: Array<{ tag_slug: string; title: string }> })
          .rows;
    for (const row of rows) {
      (samplesByTag[row.tag_slug] ??= []).push(row.title);
    }
  }

  return (
    <section>
      <h2>Tag vocabulary</h2>
      <p className="proto-dek">
        Closed vocabulary. AI picks tags from this list when classifying
        a submission. When AI confidence is below the threshold for any
        tag, the post enters the human queue. Edit the display name +
        tagline inline; merge consolidates one tag into another
        (associations move, source slug deleted); retire deletes a tag
        and cascades to its submission_tags rows. Slug is immutable —
        to rename, merge into a new slug. Every action appends to{" "}
        <code>moderation_log</code>.
      </p>

      {pendingRows.length > 0 ? (
        <>
          <h3 className="proto-h3">Pending review ({pendingRows.length})</h3>
          <p className="proto-dek">
            Ada proposed these tags during moderation. Approve to add
            them to the public vocabulary, or reject to delete the tag
            and unlink it from any submissions that picked it up. The
            moderator's tag-vocab cache is cleared on approve so Ada
            picks up the change on the next submission.
          </p>
          <table className="proto-mod-table">
            <thead>
              <tr>
                <th>Slug</th>
                <th>Name</th>
                <th>Tagline</th>
                <th>Posts (samples)</th>
                <th>Action</th>
              </tr>
            </thead>
            <tbody>
              {pendingRows.map((t) => (
                <PendingTagRow
                  key={t.slug}
                  slug={t.slug}
                  name={t.name}
                  tagline={t.tagline}
                  postCount={t.postCount}
                  sampleTitles={samplesByTag[t.slug] ?? []}
                />
              ))}
            </tbody>
          </table>
        </>
      ) : null}

      <h3 className="proto-h3">Active vocabulary</h3>
      <table className="proto-mod-table">
        <thead>
          <tr>
            <th>Slug</th>
            <th>Name</th>
            <th>Tagline</th>
            <th>Posts</th>
            <th>Action</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((t) => (
            <TagRow
              key={t.slug}
              slug={t.slug}
              name={t.name}
              tagline={t.tagline}
              posts={t.posts}
            />
          ))}
        </tbody>
      </table>

      <h3 className="proto-h3">Add new tag</h3>
      <CreateTagForm />
    </section>
  );
}
