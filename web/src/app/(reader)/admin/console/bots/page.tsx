import Link from "next/link";
import { and, count, eq, gte, isNotNull, sql } from "drizzle-orm";

import { db } from "@/db/client";
import {
  botHeartbeats,
  botReports,
  users,
} from "@/db/schema";
import { staffGate } from "@/lib/staff-gate";
import { relativeTime } from "@/lib/format";

/**
 * /admin/console/bots — index of every is_agent=true user.
 *
 * One row per bot. Stats are last 7 days unless noted. The page is
 * a single SQL pass per metric (heartbeat join, cost sum, work
 * count, open-proposal count, latest error) so it scales linearly
 * with the bot fleet, not per-bot.
 *
 * Drill links go to /admin/console/bots/[username].
 */
export default async function BotsIndexPage({
  searchParams,
}: {
  searchParams: Promise<{ as?: string }>;
}) {
  const sp = await searchParams;
  const gate = await staffGate(sp);
  if (gate) return gate;

  const sevenDaysAgo = new Date(Date.now() - 7 * 24 * 60 * 60 * 1000);

  // Bot fleet — every is_agent=true user, with their latest
  // heartbeat joined in. LEFT JOIN so bots that haven't pinged yet
  // still render (with last_seen=null).
  const bots = await db
    .select({
      id: users.id,
      username: users.username,
      role: users.role,
      botExempt: users.botModerationExempt,
      version: botHeartbeats.version,
      env: botHeartbeats.env,
      lastSeenAt: botHeartbeats.lastSeenAt,
    })
    .from(users)
    .leftJoin(botHeartbeats, eq(botHeartbeats.botId, users.id))
    .where(eq(users.isAgent, true))
    .orderBy(users.username);

  if (bots.length === 0) {
    const asSuffix = sp.as ? `?as=${sp.as}` : "";
    return (
      <section>
        <div className="proto-console-breadcrumb">
          <Link href={`/admin/console${asSuffix}`}>← Console</Link>
        </div>
        <h2>Bots</h2>
        <p className="proto-empty proto-empty-spaced">
          No agent users yet. Set <code>users.is_agent=true</code> on
          a bot account to surface it here. Bot tokens with the{" "}
          <code>bots:report</code> scope will start writing to
          /admin/console/bots/[username] as soon as they ping.
        </p>
      </section>
    );
  }

  // Aggregate stats per bot (last 7d). One query each across the
  // small bot fleet — botIds gating is unnecessary because the
  // is_agent FK confines the join space.
  const [costRows, workRows, errorRows, openProposalRows, latestReportRows] =
    await Promise.all([
      db
        .select({
          botId: botReports.botId,
          totalUsd: sql<string>`COALESCE(SUM(${botReports.costUsd}), 0)::text`,
        })
        .from(botReports)
        .where(
          and(
            gte(botReports.reportedAt, sevenDaysAgo),
            isNotNull(botReports.costUsd),
          ),
        )
        .groupBy(botReports.botId),
      db
        .select({
          botId: botReports.botId,
          n: count(),
        })
        .from(botReports)
        .where(
          and(
            eq(botReports.kind, "work_summary"),
            gte(botReports.reportedAt, sevenDaysAgo),
          ),
        )
        .groupBy(botReports.botId),
      db
        .select({
          botId: botReports.botId,
          n: count(),
        })
        .from(botReports)
        .where(
          and(
            eq(botReports.kind, "error"),
            gte(botReports.reportedAt, sevenDaysAgo),
          ),
        )
        .groupBy(botReports.botId),
      db
        .select({
          botId: botReports.botId,
          n: count(),
        })
        .from(botReports)
        .where(
          and(
            eq(botReports.kind, "proposal"),
            eq(botReports.status, "open"),
          ),
        )
        .groupBy(botReports.botId),
      // Latest report timestamp per bot — heartbeat is on
      // bot_heartbeats; this is the latest "real" report for the
      // "no work in 7d" detection.
      db
        .select({
          botId: botReports.botId,
          latest: sql<Date>`MAX(${botReports.reportedAt})`,
        })
        .from(botReports)
        .groupBy(botReports.botId),
    ]);

  const costByBot = new Map(costRows.map((r) => [r.botId, r.totalUsd]));
  const workByBot = new Map(workRows.map((r) => [r.botId, r.n]));
  const errorByBot = new Map(errorRows.map((r) => [r.botId, r.n]));
  const openProposalByBot = new Map(
    openProposalRows.map((r) => [r.botId, r.n]),
  );
  const latestByBot = new Map(latestReportRows.map((r) => [r.botId, r.latest]));

  const asSuffix = sp.as ? `?as=${sp.as}` : "";

  return (
    <section>
      <div className="proto-console-breadcrumb">
        <Link href={`/admin/console${asSuffix}`}>← Console</Link>
      </div>
      <h2>Bots</h2>
      <p className="proto-dek">
        {bots.length} agent account{bots.length === 1 ? "" : "s"}.
        Bots post heartbeats, work summaries, costs, errors, and
        proposals via <code>POST /api/v1/bots/reports</code>{" "}
        (mirrored as the <code>report_bot_status</code> MCP tool).
        Open proposals surface in the <Link href={`/admin${asSuffix}`}>Today inbox</Link>.
      </p>

      <table className="proto-mod-table">
        <thead>
          <tr>
            <th>Bot</th>
            <th>Version</th>
            <th>Last seen</th>
            <th>Latest report</th>
            <th>7d cost</th>
            <th>7d work</th>
            <th>7d errors</th>
            <th>Open proposals</th>
          </tr>
        </thead>
        <tbody>
          {bots.map((b) => {
            const cost = costByBot.get(b.id);
            const work = workByBot.get(b.id) ?? 0;
            const errors = errorByBot.get(b.id) ?? 0;
            const proposals = openProposalByBot.get(b.id) ?? 0;
            const latestReport = latestByBot.get(b.id);
            return (
              <tr
                key={b.id}
                className={
                  proposals > 0 || errors > 0
                    ? "proto-mod-row-override"
                    : undefined
                }
              >
                <td>
                  <Link href={`/admin/console/bots/${b.username}${asSuffix}`}>
                    @{b.username}
                  </Link>
                  {b.role === "system" ? (
                    <span className="proto-mod-target-type"> · system</span>
                  ) : null}
                  {b.botExempt ? (
                    <span className="proto-mod-target-type"> · mod-exempt</span>
                  ) : null}
                </td>
                <td>
                  {b.version ? (
                    <code>{b.version}</code>
                  ) : (
                    <span className="proto-meta-quiet">—</span>
                  )}
                  {b.env ? (
                    <span className="proto-meta-quiet"> · {b.env}</span>
                  ) : null}
                </td>
                <td>
                  {b.lastSeenAt ? (
                    relativeTime(b.lastSeenAt.toISOString())
                  ) : (
                    <span className="proto-meta-quiet">never</span>
                  )}
                </td>
                <td>
                  {latestReport ? (
                    relativeTime(new Date(latestReport).toISOString())
                  ) : (
                    <span className="proto-meta-quiet">none</span>
                  )}
                </td>
                <td>
                  {cost ? `$${formatUsd(cost)}` : (
                    <span className="proto-meta-quiet">—</span>
                  )}
                </td>
                <td>
                  {work || <span className="proto-meta-quiet">—</span>}
                </td>
                <td>
                  {errors > 0 ? (
                    <strong>{errors}</strong>
                  ) : (
                    <span className="proto-meta-quiet">—</span>
                  )}
                </td>
                <td>
                  {proposals > 0 ? (
                    <strong>{proposals}</strong>
                  ) : (
                    <span className="proto-meta-quiet">—</span>
                  )}
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </section>
  );
}

function formatUsd(raw: string | null): string {
  if (raw === null) return "0.00";
  const n = Number(raw);
  if (!Number.isFinite(n)) return "0.00";
  return n.toFixed(n < 1 ? 4 : 2);
}
