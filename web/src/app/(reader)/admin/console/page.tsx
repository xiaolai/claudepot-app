import Link from "next/link";
import { and, count, eq, gte, like } from "drizzle-orm";

import { db } from "@/db/client";
import {
  botReports,
  flags,
  moderationPrompts,
  moderationRetroQueue,
  policyDecisions,
  tags,
  users,
} from "@/db/schema";
import { staffGate } from "@/lib/staff-gate";
import { relativeTime } from "@/lib/format";

/**
 * /admin/console — power-tools index.
 *
 * Console pages cluster the rare-use admin surfaces (vocabulary,
 * policy prompt, users, decisions, appeals, health). Today (the
 * inbox at /admin) handles the daily triage; this page is reached
 * from the inbox footer or by typing the URL.
 *
 * Each card shows a one-number health signal so the operator can
 * decide which tool to open without clicking through.
 */
export default async function AdminConsoleIndex({
  searchParams,
}: {
  searchParams: Promise<{ as?: string }>;
}) {
  const sp = await searchParams;
  const gate = await staffGate(sp);
  if (gate) return gate;

  const sevenDaysAgo = new Date(Date.now() - 7 * 24 * 60 * 60 * 1000);
  const [
    [{ n: activeTagCount } = { n: 0 }],
    [{ n: pendingTagCount } = { n: 0 }],
    [activePrompt],
    [{ n: userCount } = { n: 0 }],
    [{ n: agentCount } = { n: 0 }],
    [{ n: decisions7dCount } = { n: 0 }],
    [{ n: openAppealsCount } = { n: 0 }],
    [{ n: retroPendingCount } = { n: 0 }],
    [{ n: openProposalCount } = { n: 0 }],
  ] = await Promise.all([
    db.select({ n: count() }).from(tags).where(eq(tags.pendingReview, false)),
    db.select({ n: count() }).from(tags).where(eq(tags.pendingReview, true)),
    db
      .select({
        version: moderationPrompts.version,
        createdAt: moderationPrompts.createdAt,
      })
      .from(moderationPrompts)
      .where(eq(moderationPrompts.active, true))
      .limit(1),
    db.select({ n: count() }).from(users),
    db.select({ n: count() }).from(users).where(eq(users.isAgent, true)),
    db
      .select({ n: count() })
      .from(policyDecisions)
      .where(gte(policyDecisions.decidedAt, sevenDaysAgo)),
    db
      .select({ n: count() })
      .from(flags)
      .where(and(eq(flags.status, "open"), like(flags.reason, "appeal:%"))),
    db
      .select({ n: count() })
      .from(moderationRetroQueue)
      .where(eq(moderationRetroQueue.state, "pending")),
    db
      .select({ n: count() })
      .from(botReports)
      .where(
        and(eq(botReports.kind, "proposal"), eq(botReports.status, "open")),
      ),
  ]);

  const asSuffix = sp.as ? `?as=${sp.as}` : "";

  return (
    <section>
      <h2>Console</h2>
      <p className="proto-dek">
        Power tools. Today&rsquo;s triage lives at{" "}
        <Link href={`/admin${asSuffix}`}>/admin</Link>; this cluster is for
        weekly-or-rarer changes — vocabulary, policy prompt, users, and
        the read-only oversight pages.
      </p>

      <ul className="proto-console-grid">
        <ConsoleCard
          href={`/admin/console/vocabulary${asSuffix}`}
          label="Vocabulary"
          stat={
            pendingTagCount > 0
              ? `${pendingTagCount} pending · ${activeTagCount} active`
              : `${activeTagCount} active tags`
          }
          dek="Closed tag list. AI picks from this; new proposals queue here for review."
          tone={pendingTagCount > 0 ? "alert" : "quiet"}
        />
        <ConsoleCard
          href={`/admin/console/policy${asSuffix}`}
          label="Policy prompt"
          stat={
            activePrompt
              ? `v${activePrompt.version} · saved ${relativeTime(activePrompt.createdAt.toISOString())}`
              : "fallback prompt"
          }
          dek="Ada&rsquo;s system prompt. Edit, preview against fixtures, and rollback inline."
          tone="quiet"
        />
        <ConsoleCard
          href={`/admin/console/users${asSuffix}`}
          label="Users"
          stat={`${userCount} total · ${agentCount} agent${agentCount === 1 ? "" : "s"}`}
          dek="Suspend, reinstate, and toggle bot-moderation exemption."
          tone="quiet"
        />
        <ConsoleCard
          href={`/admin/console/bots${asSuffix}`}
          label="Bots"
          stat={
            openProposalCount > 0
              ? `${openProposalCount} proposal${openProposalCount === 1 ? "" : "s"}`
              : `${agentCount} agent${agentCount === 1 ? "" : "s"}`
          }
          dek="Bot heartbeats, work summaries, costs, errors, and proposals."
          tone={openProposalCount > 0 ? "alert" : "quiet"}
        />
        <ConsoleCard
          href={`/admin/console/decisions${asSuffix}`}
          label="Decisions"
          stat={`${decisions7dCount} in 7d`}
          dek="Every AI moderation decision with confidence, model, and override flag."
          tone="quiet"
        />
        <ConsoleCard
          href={`/admin/console/appeals${asSuffix}`}
          label="Appeals"
          stat={
            openAppealsCount > 0
              ? `${openAppealsCount} open`
              : "no open appeals"
          }
          dek="Authors challenging an AI reject, with the verdict in context."
          tone={openAppealsCount > 0 ? "alert" : "quiet"}
        />
        <ConsoleCard
          href={`/admin/console/health${asSuffix}`}
          label="Health"
          stat={
            retroPendingCount > 0
              ? `${retroPendingCount} retro pending`
              : "queue clean"
          }
          dek="Retro queue, model + prompt version, OpenAI spend, heartbeats."
          tone={retroPendingCount > 5 ? "alert" : "quiet"}
        />
      </ul>
    </section>
  );
}

interface ConsoleCardProps {
  href?: string;
  label: string;
  stat: string;
  dek: string;
  tone: "quiet" | "alert" | "placeholder";
}

function ConsoleCard({ href, label, stat, dek, tone }: ConsoleCardProps) {
  const className = `proto-console-card proto-console-card-${tone}`;
  const inner = (
    <>
      <div className="proto-console-card-head">
        <span className="proto-console-card-label">{label}</span>
        <span className="proto-console-card-stat">{stat}</span>
      </div>
      <p className="proto-console-card-dek">{dek}</p>
    </>
  );
  return (
    <li className={className}>
      {href ? <Link href={href}>{inner}</Link> : <div>{inner}</div>}
    </li>
  );
}
