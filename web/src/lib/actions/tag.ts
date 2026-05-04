"use server";

import { revalidatePath } from "next/cache";
import { and, eq } from "drizzle-orm";
import { z } from "zod";

import { auth } from "@/lib/auth";
import { db } from "@/db/client";
import { userTagMutes } from "@/db/schema";

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
