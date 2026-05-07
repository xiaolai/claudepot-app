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
  botCostsDaily,
  botReports,
  decisionRecords,
  overrideRecords,
  providerInvoices,
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
 * Two-source read:
 *   - Closed days (yesterday and earlier UTC) come from
 *     `bot_costs_daily`, the rollup populated nightly by the
 *     daily-rollup cron (migration 0027). Survives any retention
 *     pruning of `bot_reports`.
 *   - Today (current UTC date) is computed live from `bot_reports`
 *     via the `idx_bot_reports_cost_reported` partial index, since
 *     the rollup hasn't run for today yet. This means the page's
 *     "today" column reflects every cost report posted up to the
 *     request, not just yesterday's rollup snapshot.
 *
 * Both sides UNION on the (bot_id, day) shape; the JS layer merges
 * them and computes per-day + per-bot totals.
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

  // UTC midnight that starts today's bucket. Anything >= this is
  // "today" (read live); anything < this is "closed" (read rollup).
  const today = new Date(windowEnd);
  today.setUTCHours(0, 0, 0, 0);
  const todayKey = today.toISOString().slice(0, 10);

  // Closed days from the rollup. windowStart..today (exclusive).
  const closedRows = await db
    .select({
      // bot_costs_daily.day is `date`, returned as a Date by drizzle.
      day: sql<string>`to_char(${botCostsDaily.day}, 'YYYY-MM-DD')`,
      botId: botCostsDaily.botId,
      botUsername: users.username,
      usd: sql<string>`${botCostsDaily.usd}::text`,
      reports: botCostsDaily.reports,
    })
    .from(botCostsDaily)
    .innerJoin(users, eq(users.id, botCostsDaily.botId))
    .where(
      and(
        gte(botCostsDaily.day, sql`${windowStart}::date`),
        sql`${botCostsDaily.day} < ${todayKey}::date`,
      ),
    )
    .orderBy(desc(botCostsDaily.day), users.username);

  // Today's running total from the live event log. Only fire if
  // `today >= windowStart` — for tiny windows the request might not
  // include today, and we want to keep the rollup-only path warm.
  const todayRows =
    today.getTime() >= windowStart.getTime()
      ? await db
          .select({
            day: sql<string>`${todayKey}::text`,
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
              gte(botReports.reportedAt, today),
            ),
          )
          .groupBy(botReports.botId, users.username)
      : [];

  // numeric → string (Drizzle preserves precision); parse for arithmetic.
  const detailed: BotDailyCost[] = [...todayRows, ...closedRows].map((r) => ({
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

/* ── Cost reconciliation (staff-only) ─────────────────────────── */

export interface MonthlyReconcileRow {
  /** YYYY-MM. */
  month: string;
  /** Sum of self-reported USD across all bots in that month. */
  selfReportedUsd: number;
  /** Sum of provider invoices uploaded for that month. */
  invoicedUsd: number;
  /** invoicedUsd - selfReportedUsd. Positive = under-reported. */
  varianceUsd: number;
  /** Per-provider invoice rows so the page can render them inline. */
  invoices: Array<{
    id: string;
    provider: string;
    invoicedUsd: number;
    notes: string | null;
    uploadedAt: Date;
    uploadedBy: string | null;
  }>;
}

/**
 * Per-month reconciliation: self-reported (from bot_costs_daily,
 * already UTC-bucketed by day) vs invoiced (from provider_invoices,
 * staff-uploaded). Returns rows newest-month-first, last `months`
 * months including the current one. The current month's
 * selfReportedUsd includes today's running live total; closed
 * months are stable.
 */
export async function getMonthlyReconcile(opts: {
  months?: number;
} = {}): Promise<MonthlyReconcileRow[]> {
  const months = Math.max(1, Math.min(24, opts.months ?? 12));
  const now = new Date();
  const cutoff = new Date(
    Date.UTC(now.getUTCFullYear(), now.getUTCMonth() - (months - 1), 1),
  );
  const cutoffMonthKey = cutoff.toISOString().slice(0, 7);

  // Closed days self-reported (rollup).
  const rollupRows = await db
    .select({
      month: sql<string>`to_char(${botCostsDaily.day}, 'YYYY-MM')`,
      sum: sql<string>`COALESCE(SUM(${botCostsDaily.usd}), 0)::text`,
    })
    .from(botCostsDaily)
    .where(gte(botCostsDaily.day, sql`${cutoff}::date`))
    .groupBy(sql`to_char(${botCostsDaily.day}, 'YYYY-MM')`);

  // Today's live self-reported (only contributes to current month).
  const todayStart = new Date(
    Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate()),
  );
  const todayMonthKey = todayStart.toISOString().slice(0, 7);
  const liveTodayRows = await db
    .select({
      sum: sql<string>`COALESCE(SUM(${botReports.costUsd}), 0)::text`,
    })
    .from(botReports)
    .where(
      and(
        eq(botReports.kind, "cost"),
        isNotNull(botReports.costUsd),
        gte(botReports.reportedAt, todayStart),
      ),
    );
  const liveTodayUsd = Number.parseFloat(liveTodayRows[0]?.sum ?? "0") || 0;

  // Provider invoices in the window.
  const invoiceRows = await db
    .select({
      id: providerInvoices.id,
      provider: providerInvoices.provider,
      month: providerInvoices.month,
      invoicedUsd: providerInvoices.invoicedUsd,
      uploadedAt: providerInvoices.uploadedAt,
      uploadedByUsername: users.username,
      notes: providerInvoices.notes,
    })
    .from(providerInvoices)
    .leftJoin(users, eq(users.id, providerInvoices.uploadedBy))
    .where(gte(providerInvoices.month, cutoffMonthKey))
    .orderBy(desc(providerInvoices.month), providerInvoices.provider);

  // Build the (month → row) map. Iterate over the requested window
  // so empty months still render.
  const byMonth = new Map<string, MonthlyReconcileRow>();
  for (let i = 0; i < months; i++) {
    const d = new Date(
      Date.UTC(now.getUTCFullYear(), now.getUTCMonth() - i, 1),
    );
    const m = d.toISOString().slice(0, 7);
    byMonth.set(m, {
      month: m,
      selfReportedUsd: 0,
      invoicedUsd: 0,
      varianceUsd: 0,
      invoices: [],
    });
  }

  for (const r of rollupRows) {
    const row = byMonth.get(r.month);
    if (row) row.selfReportedUsd += Number.parseFloat(r.sum) || 0;
  }
  // Add today's running total to the current month if it's in the window.
  const cur = byMonth.get(todayMonthKey);
  if (cur) cur.selfReportedUsd += liveTodayUsd;

  for (const inv of invoiceRows) {
    const row = byMonth.get(inv.month);
    if (!row) continue;
    const usd = Number.parseFloat(inv.invoicedUsd) || 0;
    row.invoicedUsd += usd;
    row.invoices.push({
      id: inv.id,
      provider: inv.provider,
      invoicedUsd: usd,
      notes: inv.notes,
      uploadedAt: inv.uploadedAt,
      uploadedBy: inv.uploadedByUsername,
    });
  }

  for (const r of byMonth.values()) {
    r.varianceUsd = r.invoicedUsd - r.selfReportedUsd;
  }

  return Array.from(byMonth.values()).sort((a, b) =>
    a.month < b.month ? 1 : -1,
  );
}
