"use server";

import { revalidatePath } from "next/cache";
import { and, eq, sql } from "drizzle-orm";
import { z } from "zod";

import { db } from "@/db/client";
import { moderationLog, submissionTags, tags } from "@/db/schema";
import { clearTagVocabCache } from "@/lib/moderation";
import { requireStaffId } from "@/lib/staff";
import { tagSlugSchema as slugSchema } from "@/lib/tags/slug";

/** Discriminated state shape returned from every admin-tag action.
 *  Pairs with React 19's useActionState in the client form components.
 *  Types-only export is fine inside a "use server" file because types
 *  are erased at compile time; non-function value exports would throw
 *  at runtime. Caller-side initial state lives at the consumer. */
export type TagActionState = { ok: boolean; message: string };

const ok = (message: string): TagActionState => ({ ok: true, message });
const err = (message: string): TagActionState => ({ ok: false, message });

const createInput = z.object({
  slug: slugSchema,
  name: z.string().trim().min(1).max(60),
  tagline: z.string().trim().max(200).optional(),
});

const renameInput = z.object({
  slug: slugSchema,
  name: z.string().trim().min(1).max(60),
  tagline: z.string().trim().max(200).optional(),
});

const mergeInput = z.object({
  fromSlug: slugSchema,
  toSlug: slugSchema,
});

const retireInput = z.object({ slug: slugSchema });

function revalidateTagSurfaces(slugs: string[]) {
  revalidatePath("/admin/console/vocabulary");
  revalidatePath("/admin/log");
  revalidatePath("/c");
  for (const s of slugs) revalidatePath(`/c/${s}`);
}

/** Auditability shouldn't gate the actual mutation. If the log write
 *  fails — most likely an unmigrated prod DB where the new enum values
 *  haven't been applied yet (migration 0011) — we surface a console
 *  warning and let the caller's success path continue. The tag change
 *  already happened; missing one log row is a smaller harm than
 *  rolling back the whole action. */
async function logTagAction(
  staffId: string,
  // tag_create  — new tag entered the active vocabulary (manual or
  //               via approving a pending Ada-proposed tag).
  // tag_rename  — display name/tagline changed.
  // tag_merge   — associations moved, source slug deleted.
  // tag_retire  — tag deleted entirely (manual retire OR rejection
  //               of a pending Ada-proposed tag — they share the
  //               same DB effect, the note string discriminates).
  action: "tag_create" | "tag_rename" | "tag_merge" | "tag_retire",
  note: string,
) {
  try {
    await db.insert(moderationLog).values({
      staffId,
      action,
      targetType: null,
      targetId: null,
      note,
    });
  } catch (err) {
    console.warn(
      `[admin-tag] moderation_log write failed for action=${action}; ` +
        `the tag mutation already committed. ` +
        `Most likely cause: migration 0011 not applied to this DB.`,
      err,
    );
  }
}

export async function createTag(
  _prev: TagActionState,
  formData: FormData,
): Promise<TagActionState> {
  const staffId = await requireStaffId();
  if (!staffId) return err("Not authorized.");

  const parsed = createInput.safeParse({
    slug: formData.get("slug"),
    name: formData.get("name"),
    tagline: formData.get("tagline") || undefined,
  });
  if (!parsed.success) {
    return err("Invalid slug or name. Slug must be kebab-case (a-z, 0-9, hyphens).");
  }

  // .returning() reports zero rows when ON CONFLICT DO NOTHING fires —
  // that's the conflict signal we surface to the user.
  const inserted = await db
    .insert(tags)
    .values({
      slug: parsed.data.slug,
      name: parsed.data.name,
      tagline: parsed.data.tagline ?? null,
    })
    .onConflictDoNothing()
    .returning({ slug: tags.slug });

  if (inserted.length === 0) {
    return err(`Tag "${parsed.data.slug}" already exists.`);
  }

  await logTagAction(staffId, "tag_create", `slug=${parsed.data.slug}`);
  // Drop the moderator's cached vocab so Ada sees the new active
  // slug on the next call. Without this, up to 60 s of submissions
  // would still treat the slug as new and produce duplicate
  // pending-tag noise (the brand-new row gets ON CONFLICT DO
  // NOTHING'd, but pending_review state could thrash).
  clearTagVocabCache();
  revalidateTagSurfaces([parsed.data.slug]);
  return ok(`Created tag "${parsed.data.slug}".`);
}

/** Renames the display fields (name, tagline) on an existing tag.
 *  Slug is immutable — the FK from submission_tags has ON DELETE
 *  CASCADE but no ON UPDATE CASCADE, so a slug rename would orphan
 *  associations or fail referential integrity. To change a slug, use
 *  mergeTag(from → to) which moves associations explicitly. */
export async function renameTag(
  _prev: TagActionState,
  formData: FormData,
): Promise<TagActionState> {
  const staffId = await requireStaffId();
  if (!staffId) return err("Not authorized.");

  const parsed = renameInput.safeParse({
    slug: formData.get("slug"),
    name: formData.get("name"),
    tagline: formData.get("tagline") || undefined,
  });
  if (!parsed.success) return err("Invalid input.");

  const updated = await db
    .update(tags)
    .set({
      name: parsed.data.name,
      tagline: parsed.data.tagline ?? null,
    })
    .where(eq(tags.slug, parsed.data.slug))
    .returning({ slug: tags.slug });

  if (updated.length === 0) {
    return err(`Tag "${parsed.data.slug}" not found.`);
  }

  await logTagAction(
    staffId,
    "tag_rename",
    `slug=${parsed.data.slug} name="${parsed.data.name}"`,
  );
  revalidateTagSurfaces([parsed.data.slug]);
  return ok("Saved.");
}

/** Move every submission_tags row from `fromSlug` to `toSlug`, then
 *  delete the source tag. Wrapped in a transaction so a partial merge
 *  can't leave orphan associations. ON CONFLICT DO NOTHING absorbs
 *  the case where a submission already carries both source and dest
 *  tags (the (submission, tag) PK forbids duplicates). */
export async function mergeTag(
  _prev: TagActionState,
  formData: FormData,
): Promise<TagActionState> {
  const staffId = await requireStaffId();
  if (!staffId) return err("Not authorized.");

  const parsed = mergeInput.safeParse({
    fromSlug: formData.get("fromSlug"),
    toSlug: formData.get("toSlug"),
  });
  if (!parsed.success) return err("Invalid slug.");
  if (parsed.data.fromSlug === parsed.data.toSlug) {
    return err("Source and destination must differ.");
  }

  let movedCount = 0;
  const errorMessage = await db.transaction(async (tx): Promise<string | null> => {
    // Symmetric existence check on both sides: missing source means
    // the merge is a no-op; missing dest would orphan rows after delete.
    const present = await tx
      .select({ slug: tags.slug })
      .from(tags)
      .where(sql`${tags.slug} IN (${parsed.data.fromSlug}, ${parsed.data.toSlug})`);
    const slugs = new Set(present.map((r) => r.slug));
    if (!slugs.has(parsed.data.fromSlug)) {
      return `Tag "${parsed.data.fromSlug}" not found.`;
    }
    if (!slugs.has(parsed.data.toSlug)) {
      return `Tag "${parsed.data.toSlug}" not found.`;
    }

    // Re-tag every association: read source ids, then bulk-insert
    // with the literal toSlug. Two round-trips inside a transaction;
    // tag merges are rare staff ops so the simplicity is worth more
    // than collapsing to one INSERT-SELECT.
    const sources = await tx
      .select({ submissionId: submissionTags.submissionId })
      .from(submissionTags)
      .where(eq(submissionTags.tagSlug, parsed.data.fromSlug));
    if (sources.length > 0) {
      const moved = await tx
        .insert(submissionTags)
        .values(
          sources.map((s) => ({
            submissionId: s.submissionId,
            tagSlug: parsed.data.toSlug,
          })),
        )
        .onConflictDoNothing()
        .returning({ submissionId: submissionTags.submissionId });
      movedCount = moved.length;
    }
    await tx.delete(tags).where(eq(tags.slug, parsed.data.fromSlug));
    return null;
  });

  if (errorMessage) return err(errorMessage);

  await logTagAction(
    staffId,
    "tag_merge",
    `from=${parsed.data.fromSlug} to=${parsed.data.toSlug} moved=${movedCount}`,
  );
  // The fromSlug just disappeared from the active vocabulary; if
  // we don't drop the cache, Ada might keep proposing it for up
  // to 60 s and applyAiTags would resurrect it as a pending row.
  clearTagVocabCache();
  revalidateTagSurfaces([parsed.data.fromSlug, parsed.data.toSlug]);
  return ok(
    `Merged "${parsed.data.fromSlug}" into "${parsed.data.toSlug}" — ${movedCount} association${movedCount === 1 ? "" : "s"} moved.`,
  );
}

export async function retireTag(
  _prev: TagActionState,
  formData: FormData,
): Promise<TagActionState> {
  const staffId = await requireStaffId();
  if (!staffId) return err("Not authorized.");

  const parsed = retireInput.safeParse({ slug: formData.get("slug") });
  if (!parsed.success) return err("Invalid slug.");

  // Cascade on submission_tags removes associations automatically.
  // .returning reports zero rows when the tag wasn't present.
  const removed = await db
    .delete(tags)
    .where(eq(tags.slug, parsed.data.slug))
    .returning({ slug: tags.slug });

  if (removed.length === 0) {
    return err(`Tag "${parsed.data.slug}" not found.`);
  }

  await logTagAction(
    staffId,
    "tag_retire",
    `slug=${parsed.data.slug}`,
  );
  // Same reason as the merge path: the slug just left the active
  // vocabulary; force the cache so Ada doesn't keep re-proposing
  // it and recreating it as a pending row.
  clearTagVocabCache();
  revalidateTagSurfaces([parsed.data.slug]);
  return ok(`Retired tag "${parsed.data.slug}".`);
}

/**
 * Approve a pending Ada-proposed tag (migration 0022). Flip
 * pending_review=false and optionally rename / set tagline. The
 * tag-vocab cache used by the moderator is cleared so Ada can
 * pick up the newly-public tag on the very next moderate() call
 * without a 60s wait.
 *
 * Form fields:
 *   - slug (required, must reference an existing pending tag)
 *   - name (optional override; if empty, keep the placeholder)
 *   - tagline (optional)
 */
const approvePendingInput = z.object({
  slug: slugSchema,
  name: z.string().trim().min(1).max(60).optional(),
  tagline: z.string().trim().max(200).optional(),
});

export async function approvePendingTag(
  _prev: TagActionState,
  formData: FormData,
): Promise<TagActionState> {
  const staffId = await requireStaffId();
  if (!staffId) return err("Not authorized.");

  const parsed = approvePendingInput.safeParse({
    slug: formData.get("slug"),
    name: formData.get("name") || undefined,
    tagline: formData.get("tagline") || undefined,
  });
  if (!parsed.success) return err("Invalid input.");

  // .returning() reports zero rows when the WHERE doesn't match —
  // either the tag doesn't exist or it's already approved. Both
  // mean "nothing to approve" from the staff's POV.
  const updateValues: {
    pendingReview: boolean;
    name?: string;
    tagline?: string | null;
  } = { pendingReview: false };
  if (parsed.data.name !== undefined) updateValues.name = parsed.data.name;
  if (parsed.data.tagline !== undefined) {
    updateValues.tagline = parsed.data.tagline;
  }

  const updated = await db
    .update(tags)
    .set(updateValues)
    .where(and(eq(tags.slug, parsed.data.slug), eq(tags.pendingReview, true)))
    .returning({ slug: tags.slug });

  if (updated.length === 0) {
    return err(`Pending tag "${parsed.data.slug}" not found.`);
  }

  await logTagAction(
    staffId,
    "tag_create",
    `approved-pending slug=${parsed.data.slug}`,
  );
  // Drop the moderator's cached vocab so Ada sees the newly-public
  // tag immediately. Without this, up to 60s of submissions would
  // still be tagged with is_new=true on this same slug, creating
  // duplicate pending-tag noise for staff to re-approve.
  clearTagVocabCache();
  revalidateTagSurfaces([parsed.data.slug]);
  return ok(`Approved tag "${parsed.data.slug}".`);
}

/**
 * Reject a pending Ada-proposed tag (migration 0022). Deletes the
 * tag row, which cascades through submission_tags via the existing
 * FK (ON DELETE CASCADE). The submissions remain — they just lose
 * this single tag association.
 */
const rejectPendingInput = z.object({ slug: slugSchema });

export async function rejectPendingTag(
  _prev: TagActionState,
  formData: FormData,
): Promise<TagActionState> {
  const staffId = await requireStaffId();
  if (!staffId) return err("Not authorized.");

  const parsed = rejectPendingInput.safeParse({ slug: formData.get("slug") });
  if (!parsed.success) return err("Invalid slug.");

  // Only delete pending rows — guard against accidentally retiring
  // an approved tag through this path. The retireTag action handles
  // approved tags explicitly.
  const removed = await db
    .delete(tags)
    .where(and(eq(tags.slug, parsed.data.slug), eq(tags.pendingReview, true)))
    .returning({ slug: tags.slug });

  if (removed.length === 0) {
    return err(`Pending tag "${parsed.data.slug}" not found.`);
  }

  await logTagAction(
    staffId,
    "tag_retire",
    `rejected-pending slug=${parsed.data.slug}`,
  );
  revalidateTagSurfaces([parsed.data.slug]);
  return ok(`Rejected tag "${parsed.data.slug}".`);
}
