import Link from "next/link";
import { and, count, desc, eq, gte, isNotNull, sql } from "drizzle-orm";

import { db } from "@/db/client";
import {
  botHeartbeats,
  moderationLog,
  moderationPrompts,
  moderationRetroQueue,
  policyDecisions,
  users,
} from "@/db/schema";
import { staffGate } from "@/lib/staff-gate";
import { relativeTime } from "@/lib/format";
import { POLICY_MODEL } from "@/lib/moderation/types";

/**
 * /admin/console/health — operator visibility into the moderation
 * pipeline's runtime state.
 *
 * What's surfaced today:
 *   - Retro queue: pending, in-progress, failed counts + oldest
 *     pending age (covers the "is anything stuck" question).
 *   - Model: the active POLICY_MODEL constant + active prompt
 *     version + last activity timestamp from policy_decisions.
 *   - Spend: 7d and 30d cost totals from policy_decisions.cost_usd.
 *   - Activity heartbeat: latest moderation_log row (any actor) and
 *     latest policy_decisions row.
 *
 * What's NOT surfaced yet (requires a `cron_runs` table — phase 4
 * scope or later): per-cron last-run / next-run / error history.
 * The retro-queue counts are a partial proxy: a deep pending
 * backlog suggests the cron isn't draining.
 */
export default async function HealthPage({
  searchParams,
}: {
  searchParams: Promise<{ as?: string }>;
}) {
  const sp = await searchParams;
  const gate = await staffGate(sp);
  if (gate) return gate;

  const sevenDaysAgo = new Date(Date.now() - 7 * 24 * 60 * 60 * 1000);
  const thirtyDaysAgo = new Date(Date.now() - 30 * 24 * 60 * 60 * 1000);

  const [
    [{ n: retroPending } = { n: 0 }],
    [{ n: retroInProgress } = { n: 0 }],
    [{ n: retroFailed } = { n: 0 }],
    [oldestPending],
    [activePrompt],
    [latestDecision],
    [latestModAction],
    [{ s: spend7d } = { s: null }],
    [{ s: spend30d } = { s: null }],
    [{ n: decisions7d } = { n: 0 }],
  ] = await Promise.all([
    db
      .select({ n: count() })
      .from(moderationRetroQueue)
      .where(eq(moderationRetroQueue.state, "pending")),
    db
      .select({ n: count() })
      .from(moderationRetroQueue)
      .where(eq(moderationRetroQueue.state, "in_progress")),
    db
      .select({ n: count() })
      .from(moderationRetroQueue)
      .where(eq(moderationRetroQueue.state, "failed")),
    db
      .select({ enqueuedAt: moderationRetroQueue.enqueuedAt })
      .from(moderationRetroQueue)
      .where(eq(moderationRetroQueue.state, "pending"))
      .orderBy(moderationRetroQueue.enqueuedAt)
      .limit(1),
    db
      .select({
        version: moderationPrompts.version,
        createdAt: moderationPrompts.createdAt,
      })
      .from(moderationPrompts)
      .where(eq(moderationPrompts.active, true))
      .limit(1),
    db
      .select({
        decidedAt: policyDecisions.decidedAt,
        modelId: policyDecisions.modelId,
      })
      .from(policyDecisions)
      .orderBy(desc(policyDecisions.decidedAt))
      .limit(1),
    db
      .select({ createdAt: moderationLog.createdAt })
      .from(moderationLog)
      .orderBy(desc(moderationLog.createdAt))
      .limit(1),
    db
      .select({
        s: sql<string | null>`COALESCE(SUM(${policyDecisions.costUsd}), 0)::text`,
      })
      .from(policyDecisions)
      .where(
        and(
          gte(policyDecisions.decidedAt, sevenDaysAgo),
          isNotNull(policyDecisions.costUsd),
        ),
      ),
    db
      .select({
        s: sql<string | null>`COALESCE(SUM(${policyDecisions.costUsd}), 0)::text`,
      })
      .from(policyDecisions)
      .where(
        and(
          gte(policyDecisions.decidedAt, thirtyDaysAgo),
          isNotNull(policyDecisions.costUsd),
        ),
      ),
    db
      .select({ n: count() })
      .from(policyDecisions)
      .where(gte(policyDecisions.decidedAt, sevenDaysAgo)),
  ]);

  const oldestPendingAge =
    oldestPending && retroPending > 0
      ? relativeTime(oldestPending.enqueuedAt.toISOString())
      : null;

  // Bot fleet heartbeats — show every is_agent user, ordered by
  // staleness so the operator notices silent bots first.
  const botFleet = await db
    .select({
      username: users.username,
      version: botHeartbeats.version,
      env: botHeartbeats.env,
      lastSeenAt: botHeartbeats.lastSeenAt,
    })
    .from(users)
    .leftJoin(botHeartbeats, eq(botHeartbeats.botId, users.id))
    .where(eq(users.isAgent, true))
    .orderBy(sql`${botHeartbeats.lastSeenAt} ASC NULLS FIRST`);

  const moderationEnabled = process.env.MODERATION_ENABLED;
  const apiKeyConfigured = Boolean(process.env.OPENAI_API_KEY);
  const asSuffix = sp.as ? `?as=${sp.as}` : "";

  return (
    <section>
      <div className="proto-console-breadcrumb">
        <Link href={`/admin/console${asSuffix}`}>← Console</Link>
      </div>
      <h2>Health</h2>
      <p className="proto-dek">
        Pipeline state at a glance. Cron-run history will land in a
        future phase; today the retro-queue counts and the activity
        heartbeats below are the substitute &mdash; a stuck cron
        manifests as a growing pending backlog and a stale
        heartbeat.
      </p>

      <div className="proto-health-grid">
        <article className="proto-health-block">
          <h3>Retro queue</h3>
          <dl className="proto-health-dl">
            <dt>Pending</dt>
            <dd>
              <strong>{retroPending}</strong>
              {oldestPendingAge ? (
                <span className="proto-meta-quiet">
                  {" "}
                  · oldest enqueued {oldestPendingAge}
                </span>
              ) : null}
            </dd>
            <dt>In progress</dt>
            <dd>{retroInProgress}</dd>
            <dt>Failed</dt>
            <dd>
              {retroFailed}
              {retroFailed > 0 ? (
                <span className="proto-meta-quiet">
                  {" "}
                  · attempts exhausted
                </span>
              ) : null}
            </dd>
          </dl>
          <p className="proto-health-cron-note">
            Drained by{" "}
            <code>/api/cron/moderation-retro</code> every 5 min.
          </p>
        </article>

        <article className="proto-health-block">
          <h3>Model</h3>
          <dl className="proto-health-dl">
            <dt>Policy model</dt>
            <dd>
              <code>{POLICY_MODEL}</code>
              {latestDecision && latestDecision.modelId !== POLICY_MODEL ? (
                <div className="proto-meta-quiet">
                  Last call used <code>{latestDecision.modelId}</code> —
                  drift after a code change?
                </div>
              ) : null}
            </dd>
            <dt>Prompt version</dt>
            <dd>
              {activePrompt ? (
                <>
                  <code>{activePrompt.version}</code>{" "}
                  <span className="proto-meta-quiet">
                    · saved{" "}
                    {relativeTime(activePrompt.createdAt.toISOString())}
                  </span>
                </>
              ) : (
                <span className="proto-meta-quiet">
                  fallback (no DB row)
                </span>
              )}
            </dd>
            <dt>Moderation enabled</dt>
            <dd>
              <code>{moderationEnabled ?? "(unset)"}</code>
              {!apiKeyConfigured ? (
                <span className="proto-meta-quiet">
                  {" "}
                  · OPENAI_API_KEY missing
                </span>
              ) : null}
            </dd>
          </dl>
        </article>

        <article className="proto-health-block">
          <h3>Spend</h3>
          <dl className="proto-health-dl">
            <dt>7-day cost</dt>
            <dd>
              ${formatUsd(spend7d)}{" "}
              <span className="proto-meta-quiet">
                · {decisions7d} decision{decisions7d === 1 ? "" : "s"}
              </span>
            </dd>
            <dt>30-day cost</dt>
            <dd>${formatUsd(spend30d)}</dd>
          </dl>
          <p className="proto-health-cron-note">
            Computed from <code>policy_decisions.cost_usd</code>.
            Calls without recorded cost (early rows, fixtures) are
            excluded.
          </p>
        </article>

        <article className="proto-health-block">
          <h3>Heartbeats</h3>
          <dl className="proto-health-dl">
            <dt>Latest decision</dt>
            <dd>
              {latestDecision ? (
                relativeTime(latestDecision.decidedAt.toISOString())
              ) : (
                <span className="proto-meta-quiet">never</span>
              )}
            </dd>
            <dt>Latest mod action</dt>
            <dd>
              {latestModAction ? (
                relativeTime(latestModAction.createdAt.toISOString())
              ) : (
                <span className="proto-meta-quiet">never</span>
              )}
            </dd>
          </dl>
          <p className="proto-health-cron-note">
            Stale heartbeats &mdash; e.g. no decision for &gt;1h
            during normal traffic &mdash; suggest the moderate()
            path is short-circuiting.
          </p>
        </article>

        <article className="proto-health-block proto-health-block-wide">
          <h3>Bot fleet</h3>
          {botFleet.length === 0 ? (
            <p className="proto-meta-quiet">
              No <code>is_agent=true</code> users yet.
            </p>
          ) : (
            <table className="proto-mod-table">
              <thead>
                <tr>
                  <th>Bot</th>
                  <th>Version</th>
                  <th>Env</th>
                  <th>Last heartbeat</th>
                </tr>
              </thead>
              <tbody>
                {botFleet.map((b) => (
                  <tr key={b.username}>
                    <td>
                      <Link
                        href={`/admin/console/bots/${b.username}${asSuffix}`}
                      >
                        @{b.username}
                      </Link>
                    </td>
                    <td>
                      {b.version ? (
                        <code>{b.version}</code>
                      ) : (
                        <span className="proto-meta-quiet">—</span>
                      )}
                    </td>
                    <td>
                      {b.env ? (
                        <code>{b.env}</code>
                      ) : (
                        <span className="proto-meta-quiet">—</span>
                      )}
                    </td>
                    <td>
                      {b.lastSeenAt ? (
                        relativeTime(b.lastSeenAt.toISOString())
                      ) : (
                        <strong className="proto-meta-quiet">never</strong>
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
          <p className="proto-health-cron-note">
            One row per <code>users.is_agent=true</code>. Bots that
            haven&rsquo;t pinged{" "}
            <code>POST /api/v1/bots/reports</code> with{" "}
            <code>kind=heartbeat</code> show as <em>never</em>.
          </p>
        </article>
      </div>
    </section>
  );
}

function formatUsd(raw: string | null): string {
  if (raw === null || raw === undefined) return "0.00";
  const n = Number(raw);
  if (!Number.isFinite(n)) return "0.00";
  return n.toFixed(n < 1 ? 4 : 2);
}
