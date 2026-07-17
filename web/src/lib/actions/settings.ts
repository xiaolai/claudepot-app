"use server";

import { Buffer } from "node:buffer";
import { revalidatePath } from "next/cache";
import { and, eq, inArray, isNull, sql } from "drizzle-orm";
import { z } from "zod";
import { Resend } from "resend";

import { auth } from "@/lib/auth";
import { allowDataExportSend } from "@/lib/data-export-rate-limit";
import { db } from "@/db/client";
import {
  comments,
  accounts,
  saves,
  sessions,
  submissions,
  userEmailPrefs,
  apiTokens,
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

  // Use the full stable user id (without punctuation) so a retry or a
  // collision between UUID prefixes can never violate the unique email /
  // username indexes and leave deletion half-applied.
  const suffix = session.user.id.replaceAll("-", "");

  // Perform bot token revocation, bot anonymization, parent anonymization,
  // and session revocation in one transaction. The previous per-bot helper
  // committed each bot independently, so a later failure could leave a
  // partially deleted account with a still-live bot or session.
  await db.transaction(async (tx) => {
    const ownedBots = await tx
      .select({ id: users.id })
      .from(users)
      .where(
        and(
          eq(users.ownerUserId, session.user.id),
          eq(users.botKind, "citizen"),
        ),
      );
    const botIds = ownedBots.map((bot) => bot.id);
    const tokenOwnerIds = [session.user.id, ...botIds];
    await tx
      .update(apiTokens)
      .set({ revokedAt: new Date() })
      .where(
        and(
          inArray(apiTokens.userId, tokenOwnerIds),
          isNull(apiTokens.revokedAt),
        ),
      );

    if (botIds.length > 0) {
      await tx
        .update(users)
        .set({
          ownerUserId: null,
          botKind: null,
          bio: null,
          avatarUrl: null,
          image: null,
          name: null,
          updatedAt: new Date(),
        })
        .where(inArray(users.id, botIds));
    }

    await tx
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

    // Auth.js account rows contain provider access/refresh tokens. Delete
    // them rather than retaining external credentials for a deleted user.
    await tx.delete(accounts).where(eq(accounts.userId, session.user.id));
    await tx
      .delete(userEmailPrefs)
      .where(eq(userEmailPrefs.userId, session.user.id));
    await tx.delete(sessions).where(eq(sessions.userId, session.user.id));
  });

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

  // Fixed-window cap (2/UTC day) — each call below runs full-table-
  // per-user dump queries and sends a paid Resend email. Charged
  // BEFORE the dump work, like the magic-link throttle. A throttled
  // call is masked as success (same oracle-free style as the
  // magic-link path): the response is indistinguishable from a sent
  // export, the email simply doesn't arrive again today.
  const allowed = await allowDataExportSend(userId);
  if (!allowed) return { ok: true, emailed: true };

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
