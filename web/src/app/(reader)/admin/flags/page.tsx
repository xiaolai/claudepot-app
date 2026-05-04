import { sql } from "drizzle-orm";

import { db } from "@/db/client";
import { submissionTags, tags } from "@/db/schema";
import { CreateTagForm } from "@/components/prototype/admin/CreateTagForm";
import { TagRow } from "@/components/prototype/admin/TagRow";
import { staffGate } from "@/lib/staff-gate";

export default async function AdminFlags({
  searchParams,
}: {
  searchParams: Promise<{ as?: string }>;
}) {
  const sp = await searchParams;
  const gate = await staffGate(sp);
  if (gate) return gate;

  // Group counts in one round-trip instead of N+1 queries.
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
    .orderBy(tags.sortOrder);

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
