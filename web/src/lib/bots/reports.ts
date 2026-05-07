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

import { sql } from "drizzle-orm";

import { db } from "@/db/client";
import { botHeartbeats, botReports } from "@/db/schema";

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
  let costUsd: string | null = null;
  if (typeof input.costUsd === "number") {
    costUsd = input.costUsd.toFixed(6);
  } else if (input.kind === "cost" && typeof payload.usd === "number") {
    costUsd = (payload.usd as number).toFixed(6);
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
