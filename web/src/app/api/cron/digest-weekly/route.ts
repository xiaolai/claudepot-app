import { NextResponse } from "next/server";
import { Resend } from "resend";
import { and, desc, eq, gte, isNull } from "drizzle-orm";

import { db } from "@/db/client";
import {
  digestSends,
  submissions,
  userEmailPrefs,
  users,
} from "@/db/schema";
import { escapeXml as escape } from "@/lib/escape-xml";
import { buildUnsubscribeUrl } from "@/lib/email/unsubscribe";

const SITE_URL = process.env.NEXT_PUBLIC_SITE_URL ?? "https://claudepot.com";
const EMAIL_FROM = process.env.EMAIL_FROM ?? "ClauDepot <noreply@claudepot.com>";

// Bound the parallelism for outgoing email so a 1k-recipient digest
// doesn't fire 1k sockets at Resend in one tick. 10 is conservative;
// the value is not load-bearing, only the bound is.
const SEND_CONCURRENCY = 10;

/**
 * Stable ISO-8601 week key, e.g. "2026-W18". Used as the idempotency
 * partition for digest_sends. Implementation note: ISO weeks pivot on
 * Thursday and start on Monday, so this never changes mid-Sunday-cron.
 */
function isoWeekKey(d: Date): string {
  const date = new Date(
    Date.UTC(d.getUTCFullYear(), d.getUTCMonth(), d.getUTCDate()),
  );
  const dayNum = date.getUTCDay() || 7;
  date.setUTCDate(date.getUTCDate() + 4 - dayNum);
  const yearStart = new Date(Date.UTC(date.getUTCFullYear(), 0, 1));
  const weekNo = Math.ceil(
    ((date.getTime() - yearStart.getTime()) / 86_400_000 + 1) / 7,
  );
  return `${date.getUTCFullYear()}-W${String(weekNo).padStart(2, "0")}`;
}

/**
 * Sunday 12:00 UTC — top submissions of the past week to opted-in users.
 * Cron schedule lives in vercel.json. The handler is a GET (Vercel calls
 * cron routes via GET) and is gated by the Vercel cron's bearer token.
 *
 * Each email carries the RFC 8058 one-click unsubscribe header pair plus
 * a visible unsubscribe link, so Gmail's bulk-sender requirements (Feb
 * 2024) are satisfied as soon as the digest goes out at scale.
 */
export async function GET(req: Request) {
  // Audit finding 2.2 — CRON_SECRET must be mandatory in production. In dev
  // (NODE_ENV !== production) we allow unauthenticated calls so the route
  // can be hit from a browser; in prod we reject if the secret is unset
  // (fail-closed) AND if the bearer doesn't match.
  const expected = process.env.CRON_SECRET;
  const isProd = process.env.NODE_ENV === "production";
  if (isProd) {
    if (!expected) {
      return NextResponse.json(
        { error: "CRON_SECRET not configured" },
        { status: 500 },
      );
    }
    if (req.headers.get("authorization") !== `Bearer ${expected}`) {
      return NextResponse.json({ error: "unauthorized" }, { status: 401 });
    }
  } else if (expected) {
    if (req.headers.get("authorization") !== `Bearer ${expected}`) {
      return NextResponse.json({ error: "unauthorized" }, { status: 401 });
    }
  }

  const apiKey = process.env.RESEND_API_KEY;
  if (!apiKey) {
    return NextResponse.json(
      { skipped: true, reason: "RESEND_API_KEY missing" },
      { status: 200 },
    );
  }
  if (!process.env.AUTH_SECRET) {
    // Without AUTH_SECRET we cannot sign per-recipient unsubscribe
    // tokens, and bulk mail without a valid List-Unsubscribe header is
    // a deliverability landmine. Fail loud rather than silently send
    // unmark-able mail.
    return NextResponse.json(
      { skipped: true, reason: "AUTH_SECRET missing — required for unsubscribe tokens" },
      { status: 500 },
    );
  }
  const resend = new Resend(apiKey);

  // Top 10 of the past week.
  const weekAgo = new Date(Date.now() - 7 * 86_400_000);
  const top = await db
    .select({
      id: submissions.id,
      title: submissions.title,
      url: submissions.url,
      score: submissions.score,
    })
    .from(submissions)
    .where(
      and(
        eq(submissions.state, "approved"),
        isNull(submissions.deletedAt),
        gte(submissions.createdAt, weekAgo),
      ),
    )
    .orderBy(desc(submissions.score))
    .limit(10);

  // Users opted into the weekly digest.
  const recipients = await db
    .select({
      id: users.id,
      email: users.email,
      username: users.username,
    })
    .from(userEmailPrefs)
    .innerJoin(users, eq(users.id, userEmailPrefs.userId))
    .where(eq(userEmailPrefs.digestWeekly, true));

  if (top.length === 0 || recipients.length === 0) {
    return NextResponse.json({
      sent: 0,
      items: top.length,
      recipients: recipients.length,
    });
  }

  // Idempotency: skip recipients who already have a (user, week) row.
  // The row is INSERTED only AFTER a successful send (see send loop
  // below) so a crash mid-loop never marks a user as sent without
  // their email actually going out. The trade-off: if we crash AFTER
  // delivery but BEFORE the insert, that user will get a second copy
  // on the next run. One duplicate per crash is strictly better than
  // a silent skip — the user can unsubscribe; they cannot retroactively
  // request a missing digest.
  const weekKey = isoWeekKey(new Date());
  const alreadySent = await db
    .select({ userId: digestSends.userId })
    .from(digestSends)
    .where(eq(digestSends.weekKey, weekKey));
  const alreadySentIds = new Set(alreadySent.map((r) => r.userId));
  const toSend = recipients.filter((r) => !alreadySentIds.has(r.id));

  if (toSend.length === 0) {
    return NextResponse.json({
      sent: 0,
      skipped: recipients.length,
      items: top.length,
      recipients: recipients.length,
      weekKey,
      reason: "all recipients already received this week's digest",
    });
  }

  const topItemsHtml = top
    .map(
      (s) => `<li>
        <a href="${SITE_URL}/post/${s.id}">${escape(s.title)}</a>
        <small>(${s.score} points)</small>
      </li>`,
    )
    .join("");
  const subject = `ClauDepot · weekly digest · top ${top.length}`;

  let sent = 0;
  let failed = 0;

  // Chunk into bounded-concurrency batches. Each batch is a Promise.all
  // so a slow recipient doesn't queue up behind a fast one, but we
  // never fan out beyond SEND_CONCURRENCY in flight at once.
  for (let i = 0; i < toSend.length; i += SEND_CONCURRENCY) {
    const batch = toSend.slice(i, i + SEND_CONCURRENCY);
    const results = await Promise.all(
      batch.map(async (r) => {
        const unsubUrl = buildUnsubscribeUrl(SITE_URL, r.id);
        if (!unsubUrl) return { ok: false, id: r.id, username: r.username };

        const html = `
          <h2>This week on ClauDepot</h2>
          <ol>${topItemsHtml}</ol>
          <p><a href="${SITE_URL}/settings">Email preferences</a> ·
            <a href="${unsubUrl}">Unsubscribe from this digest</a></p>
        `;

        try {
          await resend.emails.send({
            from: EMAIL_FROM,
            to: r.email,
            subject,
            html,
            headers: {
              // RFC 2369 + RFC 8058. Pair both headers — Gmail and
              // Apple Mail require the -Post variant for the one-click
              // experience; bare List-Unsubscribe alone is no longer
              // sufficient under Gmail's Feb 2024 bulk-sender rules.
              "List-Unsubscribe": `<${unsubUrl}>`,
              "List-Unsubscribe-Post": "List-Unsubscribe=One-Click",
            },
          });
          // Persist the dedup row only after Resend confirms accept.
          // ON CONFLICT DO NOTHING absorbs the case where another
          // concurrent run already wrote the same key.
          try {
            await db
              .insert(digestSends)
              .values({ userId: r.id, weekKey })
              .onConflictDoNothing();
          } catch (markErr) {
            // The email went out — don't fail the whole task because
            // the dedup write hiccuped. Worst case is a possible
            // duplicate on the next run, surfaced via the cron's JSON.
            console.error(
              `digest dedup-mark failed for @${r.username}:`,
              markErr,
            );
          }
          return { ok: true, id: r.id, username: r.username };
        } catch (err) {
          console.error(`digest send failed for @${r.username}:`, err);
          return { ok: false, id: r.id, username: r.username };
        }
      }),
    );
    for (const result of results) {
      if (result.ok) sent++;
      else failed++;
    }
  }

  return NextResponse.json({
    sent,
    failed,
    skipped: recipients.length - toSend.length,
    items: top.length,
    recipients: recipients.length,
    weekKey,
  });
}
