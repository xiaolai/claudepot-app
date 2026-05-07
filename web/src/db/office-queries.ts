/**
 * Office-specific read queries — drive the /office/ public window per
 * editorial/transparency.md. These read decision_records / override_records
 * / scout_runs and shape them for the UI.
 *
 * Bot-side WRITES happen from the claudepot-office private repo on
 * mac-mini-home; this file only reads.
 */

import { and, desc, eq, gte, isNotNull, sql } from "drizzle-orm";

import { db } from "./client";
import {
  botReports,
  decisionRecords,
  overrideRecords,
  submissions,
  users,
} from "./schema";

export interface OfficeDecision {
  id: string;
  submissionId: string;
  submissionTitle: string;
  submissionUrl: string | null;
  submissionType: string;
  appliedPersona: string;
  perCriterionScores: Record<string, number>;
  weightedTotal: number;
  hardRejectsHit: string[];
  inclusionGates: Record<string, boolean>;
  typeInferred: string;
  subSegmentInferred: string;
  confidence: "high" | "low";
  oneLineWhy: string;
  finalDecision: "accept" | "reject" | "borderline_to_human_queue";
  routing: "feed" | "firehose" | "human_queue";
  rubricVersion: string;
  audienceDocVersion: string;
  modelId: string;
  scoredAt: Date;
}

const OFFICE_DECISION_SELECT = {
  id: decisionRecords.id,
  submissionId: decisionRecords.submissionId,
  submissionTitle: submissions.title,
  submissionUrl: submissions.url,
  submissionType: sql<string>`${submissions.type}::text`,
  appliedPersona: decisionRecords.appliedPersona,
  perCriterionScores: decisionRecords.perCriterionScores,
  weightedTotal: decisionRecords.weightedTotal,
  hardRejectsHit: decisionRecords.hardRejectsHit,
  inclusionGates: decisionRecords.inclusionGates,
  typeInferred: sql<string>`${decisionRecords.typeInferred}::text`,
  subSegmentInferred: decisionRecords.subSegmentInferred,
  confidence: decisionRecords.confidence,
  oneLineWhy: decisionRecords.oneLineWhy,
  finalDecision: decisionRecords.finalDecision,
  routing: decisionRecords.routing,
  rubricVersion: decisionRecords.rubricVersion,
  audienceDocVersion: decisionRecords.audienceDocVersion,
  modelId: decisionRecords.modelId,
  scoredAt: decisionRecords.scoredAt,
};

function mapRow(r: {
  id: string;
  submissionId: string;
  submissionTitle: string;
  submissionUrl: string | null;
  submissionType: string;
  appliedPersona: string;
  perCriterionScores: unknown;
  weightedTotal: string; // numeric → string
  hardRejectsHit: unknown;
  inclusionGates: unknown;
  typeInferred: string;
  subSegmentInferred: string;
  confidence: "high" | "low";
  oneLineWhy: string;
  finalDecision: "accept" | "reject" | "borderline_to_human_queue";
  routing: "feed" | "firehose" | "human_queue";
  rubricVersion: string;
  audienceDocVersion: string;
  modelId: string;
  scoredAt: Date;
}): OfficeDecision {
  return {
    ...r,
    weightedTotal: Number.parseFloat(r.weightedTotal),
    perCriterionScores: r.perCriterionScores as Record<string, number>,
    hardRejectsHit: r.hardRejectsHit as string[],
    inclusionGates: r.inclusionGates as Record<string, boolean>,
  };
}

/** Recent decisions across all routings (default: accepted only). */
export async function getRecentDecisions(opts: {
  routing?: "feed" | "firehose" | "human_queue";
  persona?: string;
  limit?: number;
} = {}): Promise<OfficeDecision[]> {
  const limit = Math.min(opts.limit ?? 30, 200);
  const filters = [];
  if (opts.routing) filters.push(eq(decisionRecords.routing, opts.routing));
  if (opts.persona) filters.push(eq(decisionRecords.appliedPersona, opts.persona));

  const rows = await db
    .select(OFFICE_DECISION_SELECT)
    .from(decisionRecords)
    .innerJoin(submissions, eq(decisionRecords.submissionId, submissions.id))
    .where(filters.length > 0 ? and(...filters) : undefined)
    .orderBy(desc(decisionRecords.scoredAt))
    .limit(limit);

  return rows.map(mapRow);
}

export async function getOfficeDecisionById(
  id: string
): Promise<OfficeDecision | null> {
  const rows = await db
    .select(OFFICE_DECISION_SELECT)
    .from(decisionRecords)
    .innerJoin(submissions, eq(decisionRecords.submissionId, submissions.id))
    .where(eq(decisionRecords.id, id))
    .limit(1);
  if (rows.length === 0) return null;
  return mapRow(rows[0]);
}

/** Per-persona stats: total decisions, accept rate, avg score on accepts. */
export interface PersonaStats {
  persona: string;
  total: number;
  accepted: number;
  rejected: number;
  borderline: number;
  avgWeightedTotalAccepted: number;
}

export async function getPersonaStats(persona: string): Promise<PersonaStats> {
  const rows = await db
    .select({
      total: sql<number>`COUNT(*)::int`,
      accepted: sql<number>`COUNT(*) FILTER (WHERE ${decisionRecords.finalDecision} = 'accept')::int`,
      rejected: sql<number>`COUNT(*) FILTER (WHERE ${decisionRecords.finalDecision} = 'reject')::int`,
      borderline: sql<number>`COUNT(*) FILTER (WHERE ${decisionRecords.finalDecision} = 'borderline_to_human_queue')::int`,
      avgWeightedTotalAccepted: sql<number>`COALESCE(AVG(CASE WHEN ${decisionRecords.finalDecision} = 'accept' THEN ${decisionRecords.weightedTotal} END), 0)::float`,
    })
    .from(decisionRecords)
    .where(eq(decisionRecords.appliedPersona, persona));
  const r = rows[0] ?? { total: 0, accepted: 0, rejected: 0, borderline: 0, avgWeightedTotalAccepted: 0 };
  return { persona, ...r };
}

/* ── Bot cost reports ──────────────────────────────────────────── */

export interface BotDailyCost {
  /** ISO date (YYYY-MM-DD) in UTC. */
  day: string;
  /** Bot user.id. */
  botId: string;
  /** Bot user.username. Used for the public column header. */
  botUsername: string;
  /** Sum of cost_usd across all kind='cost' reports on that day. */
  usd: number;
  /** Number of cost reports the bot filed that day. */
  reports: number;
}

export interface BotCostSummary {
  /** Per-(bot, day) rows over the requested window, newest day first. */
  rows: BotDailyCost[];
  /** Per-day totals across all bots — for the "totals" row in the UI. */
  totalsByDay: Array<{ day: string; usd: number }>;
  /** Per-bot totals across the window — for sort-stable bot column order. */
  totalsByBot: Array<{ botId: string; botUsername: string; usd: number }>;
  /** Window edges. */
  windowStart: Date;
  windowEnd: Date;
  /** Days requested. */
  windowDays: number;
}

const DEFAULT_COST_WINDOW_DAYS = 30;
const MAX_COST_WINDOW_DAYS = 90;

/**
 * Daily cost aggregate per bot for the last N days.
 *
 * Reads `bot_reports` directly — no rollup table. The
 * idx_bot_reports_cost_reported partial index covers the predicate
 * (cost_usd IS NOT NULL) so the scan is bounded by daily report
 * volume × N days, not by the full table. Each bot files 1–10 cost
 * reports per day; over 90 days that's at most 900 rows per bot,
 * which the SUM() collapses to ~90 result rows. Cheap.
 *
 * Uses `date_trunc('day', reported_at AT TIME ZONE 'UTC')` for the
 * day bucket — matches the daily-rollup cron's UTC convention.
 *
 * Self-reported numbers. The Office page banner explicitly says so;
 * monthly reconciliation against provider invoices happens
 * privately. See lib/bots/schemas.ts:costPayloadSchema for the
 * payload shape bots POST.
 */
export async function getBotDailyCosts(opts: {
  days?: number;
} = {}): Promise<BotCostSummary> {
  const requested = opts.days ?? DEFAULT_COST_WINDOW_DAYS;
  const days = Math.max(
    1,
    Math.min(MAX_COST_WINDOW_DAYS, Math.floor(requested) || DEFAULT_COST_WINDOW_DAYS),
  );
  const windowEnd = new Date();
  const windowStart = new Date(windowEnd.getTime() - days * 86_400_000);

  const rows = await db
    .select({
      day: sql<string>`to_char(date_trunc('day', ${botReports.reportedAt} AT TIME ZONE 'UTC'), 'YYYY-MM-DD')`,
      botId: botReports.botId,
      botUsername: users.username,
      usd: sql<string>`COALESCE(SUM(${botReports.costUsd}), 0)::text`,
      reports: sql<number>`COUNT(*)::int`,
    })
    .from(botReports)
    .innerJoin(users, eq(users.id, botReports.botId))
    .where(
      and(
        eq(botReports.kind, "cost"),
        isNotNull(botReports.costUsd),
        gte(botReports.reportedAt, windowStart),
      ),
    )
    .groupBy(
      sql`date_trunc('day', ${botReports.reportedAt} AT TIME ZONE 'UTC')`,
      botReports.botId,
      users.username,
    )
    .orderBy(
      desc(sql`date_trunc('day', ${botReports.reportedAt} AT TIME ZONE 'UTC')`),
      users.username,
    );

  // numeric → string (Drizzle preserves precision); parse for arithmetic.
  const detailed: BotDailyCost[] = rows.map((r) => ({
    day: r.day,
    botId: r.botId,
    botUsername: r.botUsername,
    usd: Number.parseFloat(r.usd) || 0,
    reports: r.reports,
  }));

  const totalsByDayMap = new Map<string, number>();
  const totalsByBotMap = new Map<string, { username: string; usd: number }>();
  for (const r of detailed) {
    totalsByDayMap.set(r.day, (totalsByDayMap.get(r.day) ?? 0) + r.usd);
    const prev = totalsByBotMap.get(r.botId);
    totalsByBotMap.set(r.botId, {
      username: r.botUsername,
      usd: (prev?.usd ?? 0) + r.usd,
    });
  }

  return {
    rows: detailed,
    totalsByDay: Array.from(totalsByDayMap, ([day, usd]) => ({ day, usd })),
    totalsByBot: Array.from(totalsByBotMap, ([botId, { username, usd }]) => ({
      botId,
      botUsername: username,
      usd,
    })).sort((a, b) => b.usd - a.usd),
    windowStart,
    windowEnd,
    windowDays: days,
  };
}
