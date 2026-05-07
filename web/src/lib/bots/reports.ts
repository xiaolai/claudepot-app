/**
 * Persistence for POST /api/v1/bots/reports.
 *
 * Two write paths:
 *
 *   - heartbeat → UPSERT one row in bot_heartbeats keyed by bot_id.
 *     Returns the new last_seen_at for the response. Doesn't charge
 *     the rate-limit bucket — heartbeats are intentionally cheap.
 *
 *   - everything else → INSERT one row in bot_reports. cost_usd is
 *     denormalized out of payload (or taken from the explicit
 *     costUsd override) so the dashboard can SUM() without jsonb
 *     extraction. Proposals get status='open' so the partial unique
 *     index can dedup re-posts while one is still pending staff
 *     review.
 *
 * Uniqueness errors on proposals (bot already has an open proposal
 * with the same key) surface as { ok: false, reason: 'duplicate' };
 * callers turn that into a 409. Other DB errors propagate.
 */

import { and, eq, gte, sql } from "drizzle-orm";

import { db } from "@/db/client";
import {
  botCostsDaily,
  botHeartbeats,
  botReports,
  users,
} from "@/db/schema";

import {
  KIND_SCHEMA_BY_KIND,
  type ReportInput,
  type ReportKind,
} from "./schemas";

export type PersistResult =
  | { ok: true; kind: "heartbeat"; lastSeenAt: Date }
  | { ok: true; kind: Exclude<ReportKind, "heartbeat">; reportId: string }
  | { ok: false; reason: "validation"; detail: string }
  | { ok: false; reason: "duplicate" };

/**
 * Validate per-kind payload, then persist. The two-step parse here
 * keeps the route handler simple — it has already validated the
 * envelope (kind + payload object shape) before calling us.
 */
export async function persistBotReport(
  botId: string,
  input: ReportInput,
): Promise<PersistResult> {
  const kindSchema = KIND_SCHEMA_BY_KIND[input.kind];
  const parsed = kindSchema.safeParse(input.payload);
  if (!parsed.success) {
    const issues = parsed.error.issues
      .map((i) => `${i.path.join(".") || "(root)"}: ${i.message}`)
      .join("; ");
    return { ok: false, reason: "validation", detail: issues };
  }
  const payload = parsed.data as Record<string, unknown>;

  if (input.kind === "heartbeat") {
    const version =
      typeof payload.version === "string" ? payload.version : null;
    const env = typeof payload.env === "string" ? payload.env : null;
    const meta =
      payload.meta && typeof payload.meta === "object"
        ? (payload.meta as Record<string, unknown>)
        : null;
    // ON CONFLICT (bot_id) DO UPDATE — only the latest heartbeat
    // matters. last_seen_at refreshes on every ping.
    const [row] = await db
      .insert(botHeartbeats)
      .values({
        botId,
        version,
        env,
        meta: meta ?? undefined,
        lastSeenAt: new Date(),
      })
      .onConflictDoUpdate({
        target: botHeartbeats.botId,
        set: {
          version: sql`EXCLUDED.version`,
          env: sql`EXCLUDED.env`,
          meta: sql`EXCLUDED.meta`,
          lastSeenAt: sql`EXCLUDED.last_seen_at`,
        },
      })
      .returning({ lastSeenAt: botHeartbeats.lastSeenAt });

    return {
      ok: true,
      kind: "heartbeat",
      lastSeenAt: row?.lastSeenAt ?? new Date(),
    };
  }

  // Cost denormalization — explicit override beats payload.usd.
  // Only attaches to kind='cost' rows: every spend-aggregation path
  // (rollup cron, getBotDailyCosts, persistBotReport's cap-breach
  // check, /office/costs, /admin/console/cost-reconcile) filters on
  // kind='cost', so a costUsd value attached to e.g. a work_summary
  // would silently disappear from totals — a confusing inconsistency
  // for the bot author. Reject the override on non-cost kinds at
  // ingest instead of letting it land in the column.
  let costUsd: string | null = null;
  if (input.kind === "cost") {
    if (typeof input.costUsd === "number") {
      costUsd = input.costUsd.toFixed(6);
    } else if (typeof payload.usd === "number") {
      costUsd = (payload.usd as number).toFixed(6);
    }
  } else if (typeof input.costUsd === "number") {
    return {
      ok: false,
      reason: "validation",
      detail: `costUsd is only valid for kind='cost' (got kind='${input.kind}').`,
    };
  }

  // Proposals enter as status='open'. Everything else has status=null.
  const status = input.kind === "proposal" ? "open" : null;

  try {
    const [row] = await db
      .insert(botReports)
      .values({
        botId,
        kind: input.kind,
        payload,
        costUsd,
        status,
      })
      .returning({ id: botReports.id });

    if (!row) {
      // Should be unreachable — RETURNING on an INSERT is guaranteed
      // unless the row was eaten by a trigger we don't have. Treat
      // as validation so the caller surfaces a 422 rather than 500.
      return {
        ok: false,
        reason: "validation",
        detail: "insert returned no row",
      };
    }

    // Server-side monthly-cap-breach detection. Runs after the
    // cost-report itself is committed so a failed alert insert can
    // never block the cost from being recorded. Idempotent via the
    // partial unique index `idx_bot_reports_alert_key` — repeat
    // crosses within the same month collapse onto the same row.
    if (input.kind === "cost" && costUsd !== null) {
      // Best-effort: any failure here logs and swallows. The cost
      // report already succeeded; alerting is observability, not
      // load-bearing for the response.
      try {
        await maybeEmitCapBreachAlert(botId);
      } catch (err) {
        console.error("[bots] cap-breach detection failed", err);
      }
    }

    return {
      ok: true,
      kind: input.kind as Exclude<ReportKind, "heartbeat">,
      reportId: row.id,
    };
  } catch (e) {
    // Partial unique index `idx_bot_reports_open_proposal_key`
    // collisions surface here. Postgres raises 23505 (unique
    // violation); the Drizzle/neon-http driver wraps it as
    // PostgresError | DatabaseError. Sniff the message because the
    // adapter doesn't surface a typed code.
    const msg = e instanceof Error ? e.message : String(e);
    if (
      msg.includes("idx_bot_reports_open_proposal_key") ||
      msg.includes("duplicate key")
    ) {
      return { ok: false, reason: "duplicate" };
    }
    throw e;
  }
}

/* ── Monthly cap detection ──────────────────────────────────────── */

/**
 * If the bot has a `users.monthly_usd_cap` set and this month's
 * spend has crossed it, INSERT one alert report. Idempotent via
 * `idx_bot_reports_alert_key` — repeat crosses within the same
 * month collapse onto the same row, so the staff inbox doesn't
 * fire over and over for every additional cost report after the
 * cap is breached.
 *
 * Month-to-date is read from two sources to match the rollup
 * model: closed days come from bot_costs_daily, today comes from
 * the live bot_reports event log (since the daily-rollup cron
 * hasn't written today's bucket yet). Same as getBotDailyCosts —
 * keeping the two consistent so a Cap-breach matches what the
 * page shows.
 */
async function maybeEmitCapBreachAlert(botId: string): Promise<void> {
  const [u] = await db
    .select({ cap: users.monthlyUsdCap })
    .from(users)
    .where(eq(users.id, botId))
    .limit(1);
  const capStr = u?.cap;
  if (!capStr) return; // null cap = unlimited; no alert
  const cap = Number.parseFloat(capStr);
  if (!Number.isFinite(cap) || cap <= 0) return;

  // Month start in UTC.
  const now = new Date();
  const monthStart = new Date(
    Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), 1),
  );
  const todayStart = new Date(
    Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate()),
  );
  const monthKey = monthStart.toISOString().slice(0, 7); // YYYY-MM

  // Closed days this month (rollup).
  const closed = await db
    .select({
      sum: sql<string>`COALESCE(SUM(${botCostsDaily.usd}), 0)::text`,
    })
    .from(botCostsDaily)
    .where(
      and(
        eq(botCostsDaily.botId, botId),
        gte(botCostsDaily.day, sql`${monthStart}::date`),
        sql`${botCostsDaily.day} < ${todayStart}::date`,
      ),
    );
  const closedUsd = Number.parseFloat(closed[0]?.sum ?? "0") || 0;

  // Today live (event log).
  const live = await db
    .select({
      sum: sql<string>`COALESCE(SUM(${botReports.costUsd}), 0)::text`,
    })
    .from(botReports)
    .where(
      and(
        eq(botReports.botId, botId),
        eq(botReports.kind, "cost"),
        gte(botReports.reportedAt, todayStart),
      ),
    );
  const liveUsd = Number.parseFloat(live[0]?.sum ?? "0") || 0;

  const mtd = closedUsd + liveUsd;
  if (mtd <= cap) return;

  // Cross. Insert one alert; the partial unique index dedupes
  // repeat triggers within the month.
  await db
    .insert(botReports)
    .values({
      botId,
      kind: "alert",
      payload: {
        key: `cap_breach:${monthKey}`,
        severity: "high",
        cap,
        mtd: Number(mtd.toFixed(6)),
        message: `Monthly cap of $${cap.toFixed(2)} crossed for ${monthKey}: month-to-date $${mtd.toFixed(2)}.`,
      },
      costUsd: null,
      status: null,
    })
    .onConflictDoNothing();
}
