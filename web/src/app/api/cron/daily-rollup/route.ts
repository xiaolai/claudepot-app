import { NextResponse } from "next/server";
import { and, count, gte, lt, sql, type AnyColumn } from "drizzle-orm";

import { db } from "@/db/client";
import {
  comments,
  metricsDaily,
  submissions,
  users,
  votes,
} from "@/db/schema";

/**
 * Daily 00:00 UTC — aggregate yesterday's events into metrics_daily.
 * Idempotent (ON CONFLICT DO UPDATE) so a retry doesn't double-count.
 */
export async function GET(req: Request) {
  // Audit finding 2.2 — CRON_SECRET fail-closed in production.
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

  const now = new Date();
  // Yesterday's UTC bounds.
  const yesterdayStart = new Date(
    Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate() - 1),
  );
  const todayStart = new Date(
    Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate()),
  );
  const dayKey = yesterdayStart.toISOString().slice(0, 10);

  const range = (col: AnyColumn) =>
    and(gte(col, yesterdayStart), lt(col, todayStart));

  const [subs] = await db
    .select({ n: count() })
    .from(submissions)
    .where(range(submissions.createdAt));
  const [coms] = await db
    .select({ n: count() })
    .from(comments)
    .where(range(comments.createdAt));
  const [vts] = await db
    .select({ n: count() })
    .from(votes)
    .where(range(votes.createdAt));
  const [signs] = await db
    .select({ n: count() })
    .from(users)
    .where(range(users.createdAt));

  // Active users in last 24h: users who posted, commented, or voted.
  const since24h = new Date(now.getTime() - 86_400_000);
  const [active] = await db.execute(sql`
    SELECT COUNT(DISTINCT actor)::int AS n FROM (
      SELECT author_id AS actor FROM ${submissions} WHERE created_at >= ${since24h}
      UNION
      SELECT author_id FROM ${comments} WHERE created_at >= ${since24h}
      UNION
      SELECT user_id FROM ${votes} WHERE created_at >= ${since24h}
    ) t
  `) as unknown as Array<{ n: number }>;

  await db
    .insert(metricsDaily)
    .values({
      day: dayKey,
      submissionsTotal: subs?.n ?? 0,
      commentsTotal: coms?.n ?? 0,
      votesTotal: vts?.n ?? 0,
      signupsTotal: signs?.n ?? 0,
      activeUsers24h: Number(active?.n ?? 0),
    })
    .onConflictDoUpdate({
      target: metricsDaily.day,
      set: {
        submissionsTotal: subs?.n ?? 0,
        commentsTotal: coms?.n ?? 0,
        votesTotal: vts?.n ?? 0,
        signupsTotal: signs?.n ?? 0,
        activeUsers24h: Number(active?.n ?? 0),
      },
    });

  return NextResponse.json({
    day: dayKey,
    submissions: subs?.n ?? 0,
    comments: coms?.n ?? 0,
    votes: vts?.n ?? 0,
    signups: signs?.n ?? 0,
    active24h: Number(active?.n ?? 0),
  });
}
