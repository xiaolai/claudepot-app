"use server";

import { Buffer } from "node:buffer";
import { revalidatePath } from "next/cache";
import { eq, sql } from "drizzle-orm";
import { z } from "zod";
import { Resend } from "resend";

import { auth } from "@/lib/auth";
import { db } from "@/db/client";
import {
  comments,
  saves,
  sessions,
  submissions,
  userEmailPrefs,
  users,
  votes,
} from "@/db/schema";

/* ── Email preferences ─────────────────────────────────────────── */

const prefsInput = z.object({
  digestWeekly: z.boolean(),
  notifyReplies: z.boolean(),
});

export async function updateEmailPrefs(
  input: unknown,
): Promise<{ ok: true } | { ok: false; reason: "unauth" | "validation" }> {
  const session = await auth();
  if (!session?.user?.id) return { ok: false, reason: "unauth" };

  const parsed = prefsInput.safeParse(input);
  if (!parsed.success) return { ok: false, reason: "validation" };

  await db
    .insert(userEmailPrefs)
    .values({
      userId: session.user.id,
      digestWeekly: parsed.data.digestWeekly,
      notifyReplies: parsed.data.notifyReplies,
    })
    .onConflictDoUpdate({
      target: userEmailPrefs.userId,
      set: {
        digestWeekly: parsed.data.digestWeekly,
        notifyReplies: parsed.data.notifyReplies,
        updatedAt: new Date(),
      },
    });
  revalidatePath("/settings");
  return { ok: true };
}

/* ── Account deletion (anonymizes user; preserves their content) ── */
//
// Audit finding 2.3 — required typed confirmation before destruction.

const DELETION_CONFIRMATION = "delete my account";

const deletionInput = z.object({
  confirmation: z.string(),
});

export async function requestAccountDeletion(
  input: unknown,
): Promise<
  { ok: true } | { ok: false; reason: "unauth" | "validation" | "wrong_confirmation" }
> {
  const session = await auth();
  if (!session?.user?.id) return { ok: false, reason: "unauth" };

  const parsed = deletionInput.safeParse(input);
  if (!parsed.success) return { ok: false, reason: "validation" };
  if (parsed.data.confirmation.trim().toLowerCase() !== DELETION_CONFIRMATION) {
    return { ok: false, reason: "wrong_confirmation" };
  }

  const suffix = session.user.id.slice(0, 6);
  await db
    .update(users)
    .set({
      username: sql`'deleted_' || ${suffix}`,
      email: sql`'deleted_' || ${suffix} || '@deleted.local'`,
      bio: null,
      avatarUrl: null,
      image: null,
      name: sql`'[deleted]'`,
      role: "locked",
    })
    .where(eq(users.id, session.user.id));

  // Revoke all sessions for this user.
  await db.delete(sessions).where(eq(sessions.userId, session.user.id));

  return { ok: true };
}

/* ── Data export ────────────────────────────────────────────────
 * Audit finding 4.1 — was a no-op stub; now implemented synchronously.
 * For v1 user volumes, a single user's data fits in a JSON blob ≤1 MB
 * easily; emailing it as an attachment is the simplest path. When per-user
 * data grows past ~5 MB, move to a queue table + cron worker.
 */

const EMAIL_FROM = process.env.EMAIL_FROM ?? "ClauDepot <noreply@claudepot.com>";

export async function requestDataExport(): Promise<
  | { ok: true; emailed: boolean }
  | { ok: false; reason: "unauth" | "no_email_provider" }
> {
  const session = await auth();
  if (!session?.user?.id) return { ok: false, reason: "unauth" };

  const userId = session.user.id;

  const [me, userSubs, userComments, userVotes, userSaves, prefs] =
    await Promise.all([
      db
        .select({
          id: users.id,
          username: users.username,
          email: users.email,
          bio: users.bio,
          karma: users.karma,
          createdAt: users.createdAt,
        })
        .from(users)
        .where(eq(users.id, userId))
        .limit(1)
        .then((rs) => rs[0]),
      db.select().from(submissions).where(eq(submissions.authorId, userId)),
      db.select().from(comments).where(eq(comments.authorId, userId)),
      db.select().from(votes).where(eq(votes.userId, userId)),
      db.select().from(saves).where(eq(saves.userId, userId)),
      db
        .select()
        .from(userEmailPrefs)
        .where(eq(userEmailPrefs.userId, userId))
        .limit(1)
        .then((rs) => rs[0] ?? null),
    ]);

  if (!me?.email) return { ok: false, reason: "unauth" };

  const dump = {
    exported_at: new Date().toISOString(),
    profile: me,
    email_preferences: prefs,
    submissions: userSubs,
    comments: userComments,
    votes: userVotes,
    saves: userSaves,
  };

  const apiKey = process.env.RESEND_API_KEY;
  if (!apiKey) {
    // No Resend configured — return the dump structure size for telemetry but
    // signal that we couldn't deliver. Caller can prompt user to retry later.
    return { ok: false, reason: "no_email_provider" };
  }

  const resend = new Resend(apiKey);
  const json = JSON.stringify(dump, null, 2);
  await resend.emails.send({
    from: EMAIL_FROM,
    to: me.email,
    subject: "Your ClauDepot data export",
    text:
      "Your data export is attached as a JSON file.\n\n" +
      `Account: @${me.username}\n` +
      `Submissions: ${userSubs.length}\n` +
      `Comments: ${userComments.length}\n` +
      `Votes: ${userVotes.length}\n` +
      `Saves: ${userSaves.length}\n`,
    attachments: [
      {
        filename: `claudepot-export-${me.username}.json`,
        content: Buffer.from(json, "utf-8"),
      },
    ],
  });

  return { ok: true, emailed: true };
}
