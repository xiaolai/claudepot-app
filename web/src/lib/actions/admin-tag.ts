"use server";

import { revalidatePath } from "next/cache";
import { eq, sql } from "drizzle-orm";
import { z } from "zod";

import { db } from "@/db/client";
import { moderationLog, submissionTags, tags } from "@/db/schema";
import { requireStaffId } from "@/lib/staff";

/** Discriminated state shape returned from every admin-tag action.
 *  Pairs with React 19's useActionState in the client form components.
 *  Types-only export is fine inside a "use server" file because types
 *  are erased at compile time; non-function value exports would throw
 *  at runtime. Caller-side initial state lives at the consumer. */
export type TagActionState = { ok: boolean; message: string };

const ok = (message: string): TagActionState => ({ ok: true, message });
const err = (message: string): TagActionState => ({ ok: false, message });

// kebab-case, alphanumeric + hyphens, 2..40 chars
const SLUG = /^[a-z0-9]+(?:-[a-z0-9]+)*$/;
const slugSchema = z.string().trim().min(2).max(40).regex(SLUG);

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
  revalidatePath("/admin/flags");
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
  revalidateTagSurfaces([parsed.data.slug]);
  return ok(`Retired tag "${parsed.data.slug}".`);
}
