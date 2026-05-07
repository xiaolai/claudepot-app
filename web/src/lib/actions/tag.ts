"use server";

import { revalidatePath } from "next/cache";
import { and, eq } from "drizzle-orm";
import { z } from "zod";

import { auth } from "@/lib/auth";
import { db } from "@/db/client";
import { userTagFollows, userTagMutes } from "@/db/schema";

const muteInput = z.object({
  tagSlug: z.string().min(1),
  muted: z.boolean(),
});

export async function muteTag(
  input: unknown,
): Promise<{ ok: true } | { ok: false; reason: "unauth" | "validation" }> {
  const session = await auth();
  if (!session?.user?.id) return { ok: false, reason: "unauth" };

  const parsed = muteInput.safeParse(input);
  if (!parsed.success) return { ok: false, reason: "validation" };

  if (parsed.data.muted) {
    await db
      .insert(userTagMutes)
      .values({ userId: session.user.id, tagSlug: parsed.data.tagSlug })
      .onConflictDoNothing();
  } else {
    await db
      .delete(userTagMutes)
      .where(
        and(
          eq(userTagMutes.userId, session.user.id),
          eq(userTagMutes.tagSlug, parsed.data.tagSlug),
        ),
      );
  }

  revalidatePath(`/c/${parsed.data.tagSlug}`);
  revalidatePath("/");
  return { ok: true };
}

const followInput = z.object({
  tagSlug: z.string().min(1),
  followed: z.boolean(),
});

/**
 * Follow / unfollow a tag. Idempotent on both sides — re-following an
 * already-followed tag is a no-op insert, and unfollowing an
 * unfollowed tag is a no-op delete. Mute and follow are independent
 * states (see migration 0026); a tag can be muted-and-followed (e.g.
 * a user who wants the firehose-suppressed-from-feeds but visible on
 * the tag page).
 */
export async function followTag(
  input: unknown,
): Promise<{ ok: true } | { ok: false; reason: "unauth" | "validation" }> {
  const session = await auth();
  if (!session?.user?.id) return { ok: false, reason: "unauth" };

  const parsed = followInput.safeParse(input);
  if (!parsed.success) return { ok: false, reason: "validation" };

  if (parsed.data.followed) {
    await db
      .insert(userTagFollows)
      .values({ userId: session.user.id, tagSlug: parsed.data.tagSlug })
      .onConflictDoNothing();
  } else {
    await db
      .delete(userTagFollows)
      .where(
        and(
          eq(userTagFollows.userId, session.user.id),
          eq(userTagFollows.tagSlug, parsed.data.tagSlug),
        ),
      );
  }

  revalidatePath(`/c/${parsed.data.tagSlug}`);
  return { ok: true };
}
