import { count, desc, isNull, sql } from "drizzle-orm";

import { db } from "@/db/client";
import {
  comments,
  metricsDaily,
  submissions,
  users,
  votes,
} from "@/db/schema";

export const revalidate = 3600;

export const metadata = {
  title: "Stats",
  description: "Public activity stats for sha.com.",
};

const numberFormat = new Intl.NumberFormat("en-US");

function fmtDay(day: string): string {
  // metrics_daily.day is a DATE string (YYYY-MM-DD); render as compact MMM-DD.
  const d = new Date(`${day}T00:00:00Z`);
  return d.toLocaleDateString("en-US", {
    month: "short",
    day: "numeric",
    timeZone: "UTC",
  });
}

export default async function StatsPage() {
  // Cumulative totals come from the live tables, NOT from
  // metrics_daily, so /stats is correct from day one — before any
  // cron has fired and even when historical rollups are missing or
  // out of sync.
  const [submissionTotals, commentTotals, voteTotals, userTotals] =
    await Promise.all([
      db
        .select({ n: count() })
        .from(submissions)
        .where(sql`${submissions.deletedAt} IS NULL`),
      db
        .select({ n: count() })
        .from(comments)
        .where(isNull(comments.deletedAt)),
      db.select({ n: count() }).from(votes),
      db.select({ n: count() }).from(users),
    ]);

  // Per-day breakdown still comes from metrics_daily — that table is
  // chart-friendly (one row per UTC day) and the cron rollup
  // collapses moderation-state changes that the live tables don't.
  const recent = await db
    .select({
      day: metricsDaily.day,
      submissions: metricsDaily.submissionsTotal,
      comments: metricsDaily.commentsTotal,
      votes: metricsDaily.votesTotal,
      signups: metricsDaily.signupsTotal,
      active: metricsDaily.activeUsers24h,
    })
    .from(metricsDaily)
    .orderBy(desc(metricsDaily.day))
    .limit(30);

  // Most-recent activeUsers24h is the closest thing to a live "current"
  // signal — metrics_daily is rolled up at 00:00 UTC, so this is
  // yesterday's 24h window. Acceptable for a public number; precision
  // isn't the point.
  const latestActive = recent[0]?.active ?? 0;

  const totals = {
    submissions: submissionTotals[0]?.n ?? 0,
    comments: commentTotals[0]?.n ?? 0,
    votes: voteTotals[0]?.n ?? 0,
    signups: userTotals[0]?.n ?? 0,
  };

  return (
    <section className="proto-section proto-stats">
      <h1 className="proto-stats-title">Site stats</h1>
      <p className="proto-stats-dek">
        Activity on sha.com. Cumulative totals plus the last 30 daily
        rollups, refreshed at 00:00 UTC. Cached for 1 hour.
      </p>

      <ul className="proto-stats-grid">
        <li className="proto-stats-card">
          <div className="proto-stats-metric">
            {numberFormat.format(totals.submissions)}
          </div>
          <div className="proto-stats-metric-label">Submissions</div>
        </li>
        <li className="proto-stats-card">
          <div className="proto-stats-metric">
            {numberFormat.format(totals.comments)}
          </div>
          <div className="proto-stats-metric-label">Comments</div>
        </li>
        <li className="proto-stats-card">
          <div className="proto-stats-metric">
            {numberFormat.format(totals.votes)}
          </div>
          <div className="proto-stats-metric-label">Votes</div>
        </li>
        <li className="proto-stats-card">
          <div className="proto-stats-metric">
            {numberFormat.format(totals.signups)}
          </div>
          <div className="proto-stats-metric-label">Signups</div>
        </li>
        <li className="proto-stats-card">
          <div className="proto-stats-metric">
            {numberFormat.format(latestActive)}
          </div>
          <div className="proto-stats-metric-label">Active users (24h)</div>
        </li>
      </ul>

      {recent.length > 0 ? (
        <>
          <h2 className="proto-h3">Last 30 days</h2>
          <ol className="proto-stats-toplist">
            {recent.map((r) => (
              <li key={r.day} className="proto-stats-toprow">
                <span className="proto-stats-toprow-path">{fmtDay(r.day)}</span>
                <span className="proto-stats-toprow-views">
                  {numberFormat.format(r.submissions)} subs ·{" "}
                  {numberFormat.format(r.comments)} comments ·{" "}
                  {numberFormat.format(r.votes)} votes
                </span>
              </li>
            ))}
          </ol>
        </>
      ) : (
        <p className="proto-stats-note">
          No daily rollups yet. The cron at <code>/api/cron/daily-rollup</code>{" "}
          aggregates yesterday&rsquo;s events at 00:00 UTC; numbers here will
          appear after the first run.
        </p>
      )}

      <p className="proto-stats-note">
        Page-view analytics (RUM, web vitals, country breakdown) are collected
        privately via Vercel Web Analytics for ops and not surfaced here.
      </p>
    </section>
  );
}
