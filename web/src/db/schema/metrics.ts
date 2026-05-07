/**
 * Daily metrics rollup populated by /api/cron/daily-rollup.
 */

import { date, integer, pgTable } from "drizzle-orm/pg-core";

export const metricsDaily = pgTable("metrics_daily", {
  day: date("day").primaryKey(),
  submissionsTotal: integer("submissions_total").notNull().default(0),
  commentsTotal: integer("comments_total").notNull().default(0),
  votesTotal: integer("votes_total").notNull().default(0),
  signupsTotal: integer("signups_total").notNull().default(0),
  activeUsers24h: integer("active_users_24h").notNull().default(0),
});
