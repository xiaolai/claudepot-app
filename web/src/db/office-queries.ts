/**
 * Office-specific read queries — drive the /office/ public window per
 * editorial/transparency.md. These read decision_records / override_records
 * / scout_runs and shape them for the UI.
 *
 * Bot-side WRITES happen from the claudepot-office private repo on
 * <office-host>; this file only reads.
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

  // Closed days from the rollup, summed across providers. The
  // provider column was added in migration 0029; the per-bot-per-day
  // view at /office/costs still wants one row per (bot, day), so we
  // aggregate here. The provider split is preserved for the
  // reconciliation queries that join against provider_invoices.
  const closedRows = await db
    .select({
      // bot_costs_daily.day is `date`, returned as a Date by drizzle.
      day: sql<string>`to_char(${botCostsDaily.day}, 'YYYY-MM-DD')`,
      botId: botCostsDaily.botId,
      botUsername: users.username,
      usd: sql<string>`COALESCE(SUM(${botCostsDaily.usd}), 0)::text`,
      reports: sql<number>`COALESCE(SUM(${botCostsDaily.reports}), 0)::int`,
    })
    .from(botCostsDaily)
    .innerJoin(users, eq(users.id, botCostsDaily.botId))
    .where(
      and(
        gte(botCostsDaily.day, sql`${windowStart}::date`),
        sql`${botCostsDaily.day} < ${todayKey}::date`,
      ),
    )
    .groupBy(botCostsDaily.day, botCostsDaily.botId, users.username)
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
  /** Provider name, lowercased. Special value 'unknown' = bot
   *  reported a cost without a provider tag (defended against in
   *  the cron's COALESCE). 'live-today' = current-day live
   *  bot_reports rows that haven't been rolled up yet. */
  provider: string;
  /** Sum of self-reported USD for this (month, provider). */
  selfReportedUsd: number;
  /** Sum of invoiced USD for this (month, provider). */
  invoicedUsd: number;
  /** invoicedUsd - selfReportedUsd. Positive = under-reported. */
  varianceUsd: number;
  /** Provider invoice rows for this pair (usually 0 or 1; the
   *  unique index on (provider, month) guarantees ≤1, but the type
   *  is array-shaped for the delete-form rendering loop). */
  invoices: Array<{
    id: string;
    invoicedUsd: number;
    notes: string | null;
    uploadedAt: Date;
    uploadedBy: string | null;
  }>;
}

export interface MonthTotals {
  month: string;
  selfReportedUsd: number;
  invoicedUsd: number;
  varianceUsd: number;
}

export interface ReconcileSummary {
  /** One row per (month, provider) — primary table view. Sorted
   *  newest-month-first, then provider asc. */
  rows: MonthlyReconcileRow[];
  /** Per-month aggregates summed across providers — for the
   *  "Reconciliation status" badge surfaces. */
  monthTotals: MonthTotals[];
}

/**
 * Per-(month, provider) reconciliation: self-reported (from
 * bot_costs_daily + today's live bot_reports) vs invoiced (from
 * provider_invoices, staff-uploaded).
 *
 * Returns rows for the union of (month, provider) pairs that appear
 * on either side, plus empty rows for any month in the requested
 * window that has no data on either side (so the staff scan reads
 * "missing invoice" cleanly). The current month's selfReportedUsd
 * includes today's running live total; closed months are stable.
 */
export async function getMonthlyReconcile(opts: {
  months?: number;
} = {}): Promise<ReconcileSummary> {
  const months = Math.max(1, Math.min(24, opts.months ?? 12));
  const now = new Date();
  const cutoff = new Date(
    Date.UTC(now.getUTCFullYear(), now.getUTCMonth() - (months - 1), 1),
  );
  const cutoffMonthKey = cutoff.toISOString().slice(0, 7);

  // Closed days self-reported, split per provider.
  const rollupRows = await db
    .select({
      month: sql<string>`to_char(${botCostsDaily.day}, 'YYYY-MM')`,
      provider: botCostsDaily.provider,
      sum: sql<string>`COALESCE(SUM(${botCostsDaily.usd}), 0)::text`,
    })
    .from(botCostsDaily)
    .where(gte(botCostsDaily.day, sql`${cutoff}::date`))
    .groupBy(
      sql`to_char(${botCostsDaily.day}, 'YYYY-MM')`,
      botCostsDaily.provider,
    );

  // Today's live self-reported, split per provider. We can't roll
  // these into rollup rows yet (the daily-rollup cron hasn't run),
  // but they DO need to attribute to the right provider for the
  // reconciliation totals to match an invoice the staff uploads
  // mid-month.
  const todayStart = new Date(
    Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate()),
  );
  const todayMonthKey = todayStart.toISOString().slice(0, 7);
  const liveTodayRows = await db
    .select({
      provider: sql<string>`COALESCE(NULLIF(${botReports.payload}->>'provider', ''), 'unknown')`,
      sum: sql<string>`COALESCE(SUM(${botReports.costUsd}), 0)::text`,
    })
    .from(botReports)
    .where(
      and(
        eq(botReports.kind, "cost"),
        isNotNull(botReports.costUsd),
        gte(botReports.reportedAt, todayStart),
      ),
    )
    .groupBy(sql`COALESCE(NULLIF(${botReports.payload}->>'provider', ''), 'unknown')`);

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
    .where(gte(providerInvoices.month, cutoffMonthKey));

  // Build the row map keyed by `${month}|${provider}`.
  const key = (m: string, p: string) => `${m}|${p}`;
  const byPair = new Map<string, MonthlyReconcileRow>();
  function ensureRow(month: string, provider: string): MonthlyReconcileRow {
    const k = key(month, provider);
    let r = byPair.get(k);
    if (!r) {
      r = {
        month,
        provider,
        selfReportedUsd: 0,
        invoicedUsd: 0,
        varianceUsd: 0,
        invoices: [],
      };
      byPair.set(k, r);
    }
    return r;
  }

  for (const r of rollupRows) {
    ensureRow(r.month, r.provider).selfReportedUsd +=
      Number.parseFloat(r.sum) || 0;
  }
  for (const r of liveTodayRows) {
    ensureRow(todayMonthKey, r.provider).selfReportedUsd +=
      Number.parseFloat(r.sum) || 0;
  }
  for (const inv of invoiceRows) {
    const row = ensureRow(inv.month, inv.provider);
    const usd = Number.parseFloat(inv.invoicedUsd) || 0;
    row.invoicedUsd += usd;
    row.invoices.push({
      id: inv.id,
      invoicedUsd: usd,
      notes: inv.notes,
      uploadedAt: inv.uploadedAt,
      uploadedBy: inv.uploadedByUsername,
    });
  }
  for (const r of byPair.values()) {
    r.varianceUsd = r.invoicedUsd - r.selfReportedUsd;
  }

  // Per-month totals (across providers) for the badge surfaces.
  const monthMap = new Map<string, MonthTotals>();
  for (let i = 0; i < months; i++) {
    const d = new Date(
      Date.UTC(now.getUTCFullYear(), now.getUTCMonth() - i, 1),
    );
    const m = d.toISOString().slice(0, 7);
    monthMap.set(m, {
      month: m,
      selfReportedUsd: 0,
      invoicedUsd: 0,
      varianceUsd: 0,
    });
  }
  for (const r of byPair.values()) {
    const t = monthMap.get(r.month);
    if (!t) continue;
    t.selfReportedUsd += r.selfReportedUsd;
    t.invoicedUsd += r.invoicedUsd;
  }
  for (const t of monthMap.values()) {
    t.varianceUsd = t.invoicedUsd - t.selfReportedUsd;
  }

  return {
    rows: Array.from(byPair.values()).sort((a, b) => {
      if (a.month !== b.month) return a.month < b.month ? 1 : -1;
      return a.provider < b.provider ? -1 : 1;
    }),
    monthTotals: Array.from(monthMap.values()).sort((a, b) =>
      a.month < b.month ? 1 : -1,
    ),
  };
}

/* ── Public reconciliation summary (for /office/costs banner) ─── */

export type ReconcileStatus =
  /** Closed month, invoice present, variance ≤ 5%. */
  | "matched"
  /** Closed month, invoice present, variance > 5%. */
  | "mismatch"
  /** Closed month with self-reported activity but no invoice yet. */
  | "awaiting"
  /** Current month or month with no activity on either side. */
  | "open";

export interface PublicReconcileMonth {
  month: string;
  selfReportedUsd: number;
  invoicedUsd: number;
  varianceUsd: number;
  status: ReconcileStatus;
}

const MISMATCH_TOLERANCE_PCT = 0.05;

/**
 * Public-page reconciliation summary — totals only, no provider
 * breakdown, no invoice details. The /office/costs banner
 * surfaces these so visitors can see at a glance whether each
 * recent closed month has been reconciled. Anything more granular
 * lives on /admin/console/cost-reconcile.
 */
export async function getPublicReconcileSummary(opts: {
  months?: number;
} = {}): Promise<PublicReconcileMonth[]> {
  const months = Math.max(1, Math.min(12, opts.months ?? 3));
  const summary = await getMonthlyReconcile({ months: months + 1 });

  const now = new Date();
  const currentMonthKey = now.toISOString().slice(0, 7);

  return summary.monthTotals
    .map((t): PublicReconcileMonth => {
      const isClosed = t.month < currentMonthKey;
      const hasActivity = t.selfReportedUsd > 0 || t.invoicedUsd > 0;
      let status: ReconcileStatus;
      if (!hasActivity) {
        status = "open";
      } else if (!isClosed) {
        status = "open";
      } else if (t.invoicedUsd === 0) {
        status = "awaiting";
      } else {
        const denom = Math.max(t.invoicedUsd, t.selfReportedUsd);
        const ratio = denom > 0 ? Math.abs(t.varianceUsd) / denom : 0;
        status = ratio <= MISMATCH_TOLERANCE_PCT ? "matched" : "mismatch";
      }
      return {
        month: t.month,
        selfReportedUsd: t.selfReportedUsd,
        invoicedUsd: t.invoicedUsd,
        varianceUsd: t.varianceUsd,
        status,
      };
    })
    .filter((m) => m.month < currentMonthKey || m.selfReportedUsd > 0)
    .slice(0, months);
}
