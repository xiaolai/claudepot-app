import { notFound } from "next/navigation";
import Link from "next/link";
import { and, desc, eq, gte, isNotNull, sql } from "drizzle-orm";

import { db } from "@/db/client";
import {
  botHeartbeats,
  botReports,
  users,
} from "@/db/schema";
import { staffGate } from "@/lib/staff-gate";
import { relativeTime } from "@/lib/format";
import { KIND_LABELS, type ReportKind } from "@/lib/bots";
import { ProposalActionButton } from "@/components/admin/ProposalActionButton";

const TIMELINE_LIMIT = 100;

/**
 * /admin/console/bots/[username] — single bot drill.
 *
 * Three blocks:
 *   - Identity + heartbeat (version, env, last_seen).
 *   - Open + recently-resolved proposals (with accept/reject inline).
 *   - Timeline of the latest 100 non-heartbeat reports.
 *   - 7d cost sparkline (daily totals).
 *
 * Hits the bot's own row only — no cross-bot data here. Stats are
 * cheap because of the (bot_id, reported_at DESC) index.
 */
export default async function BotDrillPage({
  params,
  searchParams,
}: {
  params: Promise<{ username: string }>;
  searchParams: Promise<{ as?: string }>;
}) {
  const { username } = await params;
  const sp = await searchParams;
  const gate = await staffGate(sp);
  if (gate) return gate;

  const [bot] = await db
    .select({
      id: users.id,
      username: users.username,
      role: users.role,
      botExempt: users.botModerationExempt,
      isAgent: users.isAgent,
      createdAt: users.createdAt,
    })
    .from(users)
    .where(eq(users.username, username))
    .limit(1);

  if (!bot || !bot.isAgent) notFound();

  const sevenDaysAgo = new Date(Date.now() - 7 * 24 * 60 * 60 * 1000);

  const [heartbeat, openProposals, recentResolvedProposals, timeline, dailyCost] =
    await Promise.all([
      db
        .select()
        .from(botHeartbeats)
        .where(eq(botHeartbeats.botId, bot.id))
        .limit(1)
        .then((rows) => rows[0] ?? null),
      db
        .select({
          id: botReports.id,
          payload: botReports.payload,
          reportedAt: botReports.reportedAt,
        })
        .from(botReports)
        .where(
          and(
            eq(botReports.botId, bot.id),
            eq(botReports.kind, "proposal"),
            eq(botReports.status, "open"),
          ),
        )
        .orderBy(botReports.reportedAt),
      db
        .select({
          id: botReports.id,
          payload: botReports.payload,
          status: botReports.status,
          resolvedAt: botReports.resolvedAt,
        })
        .from(botReports)
        .where(
          and(
            eq(botReports.botId, bot.id),
            eq(botReports.kind, "proposal"),
            sql`${botReports.status} IN ('accepted', 'rejected')`,
          ),
        )
        .orderBy(desc(botReports.resolvedAt))
        .limit(10),
      db
        .select({
          id: botReports.id,
          kind: botReports.kind,
          payload: botReports.payload,
          costUsd: botReports.costUsd,
          status: botReports.status,
          reportedAt: botReports.reportedAt,
        })
        .from(botReports)
        .where(eq(botReports.botId, bot.id))
        .orderBy(desc(botReports.reportedAt))
        .limit(TIMELINE_LIMIT),
      db
        .select({
          day: sql<string>`DATE_TRUNC('day', ${botReports.reportedAt})::date::text`,
          totalUsd: sql<string>`COALESCE(SUM(${botReports.costUsd}), 0)::text`,
        })
        .from(botReports)
        .where(
          and(
            eq(botReports.botId, bot.id),
            gte(botReports.reportedAt, sevenDaysAgo),
            isNotNull(botReports.costUsd),
          ),
        )
        .groupBy(sql`DATE_TRUNC('day', ${botReports.reportedAt})`)
        .orderBy(sql`DATE_TRUNC('day', ${botReports.reportedAt})`),
    ]);

  const totalCost7d = dailyCost.reduce(
    (acc, r) => acc + (Number(r.totalUsd) || 0),
    0,
  );
  const sparkMax = Math.max(1, ...dailyCost.map((r) => Number(r.totalUsd) || 0));

  const asSuffix = sp.as ? `?as=${sp.as}` : "";

  return (
    <section>
      <div className="proto-console-breadcrumb">
        <Link href={`/admin/console/bots${asSuffix}`}>← Bots</Link>
      </div>
      <h2>@{bot.username}</h2>

      <article className="proto-decision">
        <h3>Identity</h3>
        <dl className="proto-decision-meta">
          <dt>Role</dt>
          <dd>
            <code>{bot.role}</code>
            {bot.botExempt ? (
              <span className="proto-meta-quiet"> · moderation-exempt</span>
            ) : null}
          </dd>
          <dt>Joined</dt>
          <dd>{relativeTime(bot.createdAt.toISOString())}</dd>
          <dt>Version</dt>
          <dd>
            {heartbeat?.version ? (
              <code>{heartbeat.version}</code>
            ) : (
              <span className="proto-meta-quiet">never reported</span>
            )}
          </dd>
          <dt>Environment</dt>
          <dd>
            {heartbeat?.env ? (
              <code>{heartbeat.env}</code>
            ) : (
              <span className="proto-meta-quiet">—</span>
            )}
          </dd>
          <dt>Last seen</dt>
          <dd>
            {heartbeat?.lastSeenAt ? (
              relativeTime(heartbeat.lastSeenAt.toISOString())
            ) : (
              <span className="proto-meta-quiet">never</span>
            )}
          </dd>
          <dt>7d cost</dt>
          <dd>${totalCost7d.toFixed(totalCost7d < 1 ? 4 : 2)}</dd>
        </dl>
        {dailyCost.length > 0 ? (
          <div className="proto-bot-spark" aria-label="7-day cost sparkline">
            {dailyCost.map((d) => {
              const value = Number(d.totalUsd) || 0;
              const height = Math.round((value / sparkMax) * 100);
              return (
                <span
                  key={d.day}
                  className="proto-bot-spark-bar"
                  style={{ height: `${height}%` }}
                  title={`${d.day}: $${value.toFixed(value < 1 ? 4 : 2)}`}
                />
              );
            })}
          </div>
        ) : null}
      </article>

      <article className="proto-decision">
        <h3>Open proposals ({openProposals.length})</h3>
        {openProposals.length === 0 ? (
          <p className="proto-meta-quiet">
            No open proposals from this bot.
          </p>
        ) : (
          <ul className="proto-bot-proposal-list">
            {openProposals.map((p) => {
              const payload = p.payload as Record<string, unknown>;
              const subKind =
                typeof payload.kind === "string" ? payload.kind : "general";
              const reason =
                typeof payload.reason === "string" ? payload.reason : "";
              const target =
                typeof payload.target === "string" ? payload.target : null;
              return (
                <li key={p.id} className="proto-bot-proposal">
                  <div className="proto-bot-proposal-head">
                    <span className="proto-state-pill proto-state-pill-pending">
                      {subKind}
                    </span>
                    <span className="proto-meta-quiet">
                      {relativeTime(p.reportedAt.toISOString())}
                    </span>
                  </div>
                  <p className="proto-bot-proposal-reason">{reason}</p>
                  {target ? (
                    <p className="proto-bot-proposal-target">
                      <strong>Target:</strong> <code>{target}</code>
                    </p>
                  ) : null}
                  <div className="proto-bot-proposal-actions">
                    <ProposalActionButton
                      reportId={p.id}
                      action="accept"
                      className="proto-mod-btn proto-mod-btn-keep"
                      pendingLabel="Accepting…"
                    >
                      Accept
                    </ProposalActionButton>
                    <ProposalActionButton
                      reportId={p.id}
                      action="reject"
                      className="proto-mod-btn proto-mod-btn-remove"
                      pendingLabel="Rejecting…"
                    >
                      Reject
                    </ProposalActionButton>
                  </div>
                </li>
              );
            })}
          </ul>
        )}

        {recentResolvedProposals.length > 0 ? (
          <>
            <h4 className="proto-h3">Recently resolved</h4>
            <ul className="proto-bot-proposal-list-quiet">
              {recentResolvedProposals.map((p) => {
                const payload = p.payload as Record<string, unknown>;
                const subKind =
                  typeof payload.kind === "string" ? payload.kind : "general";
                const reason =
                  typeof payload.reason === "string" ? payload.reason : "";
                return (
                  <li key={p.id}>
                    <span className="proto-meta-quiet">
                      {p.resolvedAt
                        ? relativeTime(p.resolvedAt.toISOString())
                        : "—"}{" "}
                      · {p.status} · {subKind}:
                    </span>{" "}
                    {reason}
                  </li>
                );
              })}
            </ul>
          </>
        ) : null}
      </article>

      <article className="proto-decision">
        <h3>Timeline</h3>
        {timeline.length === 0 ? (
          <p className="proto-meta-quiet">No reports yet.</p>
        ) : (
          <table className="proto-mod-table">
            <thead>
              <tr>
                <th>When</th>
                <th>Kind</th>
                <th>Cost</th>
                <th>Summary</th>
              </tr>
            </thead>
            <tbody>
              {timeline.map((r) => {
                const kind = r.kind as ReportKind;
                return (
                  <tr key={r.id}>
                    <td>{relativeTime(r.reportedAt.toISOString())}</td>
                    <td>
                      <span className="proto-mod-target-type">
                        {KIND_LABELS[kind] ?? kind}
                      </span>
                    </td>
                    <td>
                      {r.costUsd ? (
                        `$${formatUsd(r.costUsd)}`
                      ) : (
                        <span className="proto-meta-quiet">—</span>
                      )}
                    </td>
                    <td className="proto-mod-reason">
                      {summarizePayload(kind, r.payload, r.status)}
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        )}
      </article>
    </section>
  );
}

function summarizePayload(
  kind: ReportKind,
  payload: unknown,
  status: string | null,
): string {
  const p = (payload ?? {}) as Record<string, unknown>;
  switch (kind) {
    case "heartbeat":
      return "(heartbeat)";
    case "work_summary": {
      const units = p.units as Record<string, number> | undefined;
      if (!units) return "—";
      const top = Object.entries(units)
        .sort((a, b) => b[1] - a[1])
        .slice(0, 3)
        .map(([k, v]) => `${k}=${v}`)
        .join(", ");
      return top || "—";
    }
    case "cost":
      return `${str(p.provider)}/${str(p.model)} · in=${num(p.inputTokens)} out=${num(p.outputTokens)}`;
    case "error":
      return `${str(p.severity)}: ${str(p.message)}`.slice(0, 200);
    case "proposal": {
      const subKind = str(p.kind);
      const reason = str(p.reason);
      const statusBit = status ? ` (${status})` : "";
      return `${subKind}: ${reason}`.slice(0, 200) + statusBit;
    }
    case "decision_summary": {
      const v = p.verdicts as Record<string, number> | undefined;
      if (!v) return "—";
      const top = Object.entries(v)
        .map(([k, n]) => `${k}=${n}`)
        .join(", ");
      const drift =
        typeof p.driftZ === "number" ? ` · driftZ=${p.driftZ.toFixed(2)}` : "";
      return top + drift;
    }
  }
}

function str(v: unknown): string {
  return typeof v === "string" ? v : "";
}

function num(v: unknown): string {
  return typeof v === "number" ? String(v) : "—";
}

function formatUsd(raw: string | null): string {
  if (raw === null) return "0.00";
  const n = Number(raw);
  if (!Number.isFinite(n)) return "0.00";
  return n.toFixed(n < 1 ? 6 : 2);
}
