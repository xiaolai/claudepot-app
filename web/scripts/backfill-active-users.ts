/**
 * One-shot backfill for `metrics_daily`.
 *
 * Why: the daily-rollup cron at /api/cron/daily-rollup had a broken
 * iterator-destructure on the active-users query
 * (`const [active] = await db.execute(...)`), fixed in ef3b858. Both
 * Neon drivers actually return a pg-style result object for raw SQL
 * with no fields, and iterator-destructuring an object throws — so
 * every cron run aborted before the INSERT, leaving `metrics_daily`
 * empty for the platform's whole history.
 *
 * What: recomputes the same five aggregates the cron writes —
 *   submissions_total, comments_total, votes_total, signups_total,
 *   active_users_24h (= COUNT(DISTINCT actor) over submissions ∪
 *   comments ∪ votes within the day's UTC window) — for every
 *   completed UTC day from the first signal in the data up to (and
 *   not including) today UTC. Upserts via ON CONFLICT DO UPDATE so
 *   reruns are idempotent.
 *
 * Usage:
 *   pnpm exec tsx --env-file=.env.local scripts/backfill-active-users.ts          # dry-run
 *   pnpm exec tsx --env-file=.env.local scripts/backfill-active-users.ts --apply  # write
 *
 * Uses the Neon HTTP driver — each statement is its own round-trip,
 * no transaction needed.
 */

import { neon } from "@neondatabase/serverless";

const apply = process.argv.includes("--apply");

const url = process.env.DATABASE_URL ?? process.env.NEON_DATABASE_URL;
if (!url) {
  console.error("missing DATABASE_URL / NEON_DATABASE_URL");
  process.exit(1);
}

const sql = neon(url);

type Bounds = {
  first_signal: string | null;
  today_utc: string;
};

const [bounds] = (await sql`
  SELECT
    LEAST(
      (SELECT MIN(created_at) FROM submissions),
      (SELECT MIN(created_at) FROM comments),
      (SELECT MIN(created_at) FROM votes),
      (SELECT MIN(created_at) FROM users)
    )::date::text AS first_signal,
    (NOW() AT TIME ZONE 'UTC')::date::text AS today_utc
`) as Bounds[];

if (!bounds.first_signal) {
  console.log("No signal in submissions / comments / votes / users — nothing to roll up.");
  process.exit(0);
}

const firstDay = bounds.first_signal;
const todayUtc = bounds.today_utc;

// Build the inclusive list of UTC days [firstDay, todayUtc).
const days: string[] = [];
{
  const start = new Date(`${firstDay}T00:00:00Z`);
  const end = new Date(`${todayUtc}T00:00:00Z`);
  for (
    let d = new Date(start);
    d.getTime() < end.getTime();
    d.setUTCDate(d.getUTCDate() + 1)
  ) {
    days.push(d.toISOString().slice(0, 10));
  }
}

if (days.length === 0) {
  console.log("No completed UTC days in range yet.");
  process.exit(0);
}

console.log(`Rolling up ${days.length} day(s): ${days[0]} … ${days.at(-1)}`);
console.log(`Mode: ${apply ? "APPLY (will UPSERT rows)" : "dry-run"}\n`);

type Aggregate = {
  submissions_total: number;
  comments_total: number;
  votes_total: number;
  signups_total: number;
  active_users_24h: number;
};

let upserted = 0;
let unchanged = 0;

for (const day of days) {
  // Single round-trip: collapse all five aggregates into one query.
  // Each scalar subquery uses the same half-open [day, day+1) window
  // the cron uses, so the results are byte-equivalent to a healthy
  // cron run.
  const dayStart = `${day} 00:00:00+00`;
  const [agg] = (await sql`
    SELECT
      (SELECT COUNT(*)::int FROM submissions
        WHERE created_at >= ${dayStart}::timestamptz
          AND created_at <  (${dayStart}::timestamptz + INTERVAL '1 day')
      ) AS submissions_total,
      (SELECT COUNT(*)::int FROM comments
        WHERE created_at >= ${dayStart}::timestamptz
          AND created_at <  (${dayStart}::timestamptz + INTERVAL '1 day')
      ) AS comments_total,
      (SELECT COUNT(*)::int FROM votes
        WHERE created_at >= ${dayStart}::timestamptz
          AND created_at <  (${dayStart}::timestamptz + INTERVAL '1 day')
      ) AS votes_total,
      (SELECT COUNT(*)::int FROM users
        WHERE created_at >= ${dayStart}::timestamptz
          AND created_at <  (${dayStart}::timestamptz + INTERVAL '1 day')
      ) AS signups_total,
      (SELECT COUNT(DISTINCT actor)::int FROM (
         SELECT author_id AS actor FROM submissions
           WHERE created_at >= ${dayStart}::timestamptz
             AND created_at <  (${dayStart}::timestamptz + INTERVAL '1 day')
         UNION
         SELECT author_id FROM comments
           WHERE created_at >= ${dayStart}::timestamptz
             AND created_at <  (${dayStart}::timestamptz + INTERVAL '1 day')
         UNION
         SELECT user_id FROM votes
           WHERE created_at >= ${dayStart}::timestamptz
             AND created_at <  (${dayStart}::timestamptz + INTERVAL '1 day')
       ) t
      ) AS active_users_24h
  `) as Aggregate[];

  // Compare against the existing row (if any).
  const existing = (await sql`
    SELECT submissions_total, comments_total, votes_total,
           signups_total, active_users_24h
      FROM metrics_daily
     WHERE day = ${day}::date
  `) as Aggregate[];

  const matches =
    existing.length === 1 &&
    existing[0].submissions_total === agg.submissions_total &&
    existing[0].comments_total === agg.comments_total &&
    existing[0].votes_total === agg.votes_total &&
    existing[0].signups_total === agg.signups_total &&
    existing[0].active_users_24h === agg.active_users_24h;

  if (matches) {
    unchanged++;
    continue;
  }

  const before = existing[0]
    ? `${existing[0].submissions_total}/${existing[0].comments_total}/${existing[0].votes_total}/${existing[0].signups_total}/${existing[0].active_users_24h}`
    : "(missing)";
  const after = `${agg.submissions_total}/${agg.comments_total}/${agg.votes_total}/${agg.signups_total}/${agg.active_users_24h}`;
  console.log(`  ${day}: ${before} → ${after}    (subs/coms/votes/signups/dau)`);

  if (apply) {
    await sql`
      INSERT INTO metrics_daily
        (day, submissions_total, comments_total, votes_total,
         signups_total, active_users_24h)
      VALUES
        (${day}::date, ${agg.submissions_total}, ${agg.comments_total},
         ${agg.votes_total}, ${agg.signups_total}, ${agg.active_users_24h})
      ON CONFLICT (day) DO UPDATE
        SET submissions_total = EXCLUDED.submissions_total,
            comments_total    = EXCLUDED.comments_total,
            votes_total       = EXCLUDED.votes_total,
            signups_total     = EXCLUDED.signups_total,
            active_users_24h  = EXCLUDED.active_users_24h
    `;
    upserted++;
  }
}

console.log(`\nSummary:`);
console.log(`  days scanned:     ${days.length}`);
console.log(`  unchanged:        ${unchanged}`);
console.log(`  needs upsert:     ${days.length - unchanged}`);
console.log(`  upserted:         ${upserted}`);
if (!apply && days.length - unchanged > 0) {
  console.log(`\nDry run only. Re-run with --apply to write.`);
}
