import Link from "next/link";
import { and, count, desc, eq, gte, inArray, isNull, like, ne } from "drizzle-orm";

import { db } from "@/db/client";
import {
  flags,
  moderationLog,
  policyDecisions,
  submissions,
} from "@/db/schema";
import { getSystemUserId } from "@/lib/moderation";

export type InboxFilter = "all" | "submission" | "flag" | "appeal";

interface Props {
  asSuffix: string;
  activeFilter: InboxFilter;
}

/**
 * Today inbox header — five stats. Each stat is a Link that
 * filters (or clears the filter on) the stream below. Counts are
 * computed server-side per request so they're always fresh; at
 * solo-admin scale this is cheaper than wiring a poller.
 *
 * "Ada agreement (7d)" approximates AI-vs-human concord: count of
 * AI rejects in the last 7d that were NOT restored by staff,
 * divided by total AI rejects in the same window. Restores by the
 * system user (Ada herself, e.g. via retro-queue) don't count as
 * disagreement; only human staff overrides do.
 */
export async function HeaderChips({ asSuffix, activeFilter }: Props) {
  const systemUserId = await getSystemUserId().catch(() => null);
  const startOfDayUtc = startOfTodayUtc();
  const sevenDaysAgo = new Date(Date.now() - 7 * 24 * 60 * 60 * 1000);

  const [
    pendingCountRow,
    openFlagsCountRow,
    appealsCountRow,
    todayActionsCountRow,
    aiRejectIds,
    restoredIds,
  ] = await Promise.all([
    db
      .select({ n: count() })
      .from(submissions)
      .where(and(eq(submissions.state, "pending"), isNull(submissions.deletedAt))),
    db
      .select({ n: count() })
      .from(flags)
      .where(
        and(
          eq(flags.status, "open"),
          // Reasons that DON'T start with 'appeal:'.
          // Drizzle has no `notLike`, so we approximate with
          // `like('appeal:%')` and subtract; cheaper to just filter
          // on the app side after fetching, but counting here keeps
          // the chip cheap. We get exact non-appeal count by
          // computing total - appeal count below.
        ),
      ),
    db
      .select({ n: count() })
      .from(flags)
      .where(and(eq(flags.status, "open"), like(flags.reason, "appeal:%"))),
    systemUserId
      ? db
          .select({ n: count() })
          .from(moderationLog)
          .where(
            and(
              ne(moderationLog.staffId, systemUserId),
              gte(moderationLog.createdAt, startOfDayUtc),
            ),
          )
      : db
          .select({ n: count() })
          .from(moderationLog)
          .where(gte(moderationLog.createdAt, startOfDayUtc)),
    // AI rejects in the last 7 days
    db
      .select({
        targetType: policyDecisions.targetType,
        targetId: policyDecisions.targetId,
      })
      .from(policyDecisions)
      .where(
        and(
          eq(policyDecisions.verdict, "reject"),
          gte(policyDecisions.decidedAt, sevenDaysAgo),
        ),
      ),
    // Restore actions by non-system staff in the last 7 days
    systemUserId
      ? db
          .select({
            targetType: moderationLog.targetType,
            targetId: moderationLog.targetId,
          })
          .from(moderationLog)
          .where(
            and(
              eq(moderationLog.action, "restore"),
              ne(moderationLog.staffId, systemUserId),
              gte(moderationLog.createdAt, sevenDaysAgo),
            ),
          )
      : db
          .select({
            targetType: moderationLog.targetType,
            targetId: moderationLog.targetId,
          })
          .from(moderationLog)
          .where(
            and(
              eq(moderationLog.action, "restore"),
              gte(moderationLog.createdAt, sevenDaysAgo),
            ),
          ),
  ]);

  const pendingCount = pendingCountRow[0]?.n ?? 0;
  const openFlagsTotal = openFlagsCountRow[0]?.n ?? 0;
  const appealsCount = appealsCountRow[0]?.n ?? 0;
  // Open flags shown as the chip exclude appeals (which have their
  // own chip).
  const openFlagsNonAppeal = Math.max(0, openFlagsTotal - appealsCount);
  const todayActions = todayActionsCountRow[0]?.n ?? 0;

  const restoredKeys = new Set(
    restoredIds.map((r) => `${r.targetType}:${r.targetId}`),
  );
  const overridden = aiRejectIds.filter((d) =>
    restoredKeys.has(`${d.targetType}:${d.targetId}`),
  ).length;
  const aiAgreement =
    aiRejectIds.length === 0
      ? null
      : Math.round(((aiRejectIds.length - overridden) / aiRejectIds.length) * 100);

  return (
    <ul className="proto-inbox-chips" aria-label="Inbox metrics">
      <Chip
        href={hrefFor("submission", asSuffix)}
        active={activeFilter === "submission"}
        label={`${pendingCount} pending`}
        title={`${pendingCount} submissions awaiting first-review`}
      />
      <Chip
        href={hrefFor("flag", asSuffix)}
        active={activeFilter === "flag"}
        label={`${openFlagsNonAppeal} open flag${openFlagsNonAppeal === 1 ? "" : "s"}`}
        title={`${openFlagsNonAppeal} community flags`}
      />
      <Chip
        href={hrefFor("appeal", asSuffix)}
        active={activeFilter === "appeal"}
        label={`${appealsCount} appeal${appealsCount === 1 ? "" : "s"}`}
        title={`${appealsCount} open appeals on AI rejects`}
      />
      <Chip
        href={null}
        label={
          aiAgreement === null
            ? "Ada agree —"
            : `Ada agree ${aiAgreement}%`
        }
        title={`Ada AI moderator agreement over the last 7d (${aiRejectIds.length} reject${aiRejectIds.length === 1 ? "" : "s"})`}
      />
      <Chip
        href={null}
        label={`${todayActions} today`}
        title={`${todayActions} staff action${todayActions === 1 ? "" : "s"} since 00:00 UTC`}
      />
      {activeFilter !== "all" ? (
        <li className="proto-inbox-chips-clear">
          <Link href={`/admin${asSuffix}`}>Clear filter</Link>
        </li>
      ) : null}
    </ul>
  );
}

function hrefFor(kind: Exclude<InboxFilter, "all">, asSuffix: string): string {
  const base = `/admin?kind=${kind}`;
  if (!asSuffix) return base;
  // asSuffix is `?as=...`; strip the leading `?` and append.
  return `${base}&${asSuffix.slice(1)}`;
}

interface ChipProps {
  href: string | null;
  active?: boolean;
  label: string;
  title: string;
}

function Chip({ href, active, label, title }: ChipProps) {
  const className = active
    ? "proto-inbox-chip proto-inbox-chip-active"
    : "proto-inbox-chip";
  if (href) {
    return (
      <li>
        <Link href={href} className={className} title={title}>
          {label}
        </Link>
      </li>
    );
  }
  return (
    <li>
      <span className={className} title={title}>
        {label}
      </span>
    </li>
  );
}

function startOfTodayUtc(): Date {
  const now = new Date();
  return new Date(
    Date.UTC(now.getUTCFullYear(), now.getUTCMonth(), now.getUTCDate()),
  );
}

// Suppress unused-import warning for `inArray` while we settle on the
// final query shape; keep available for the next iteration that joins
// per-target-type vs. id lists.
void inArray;
void desc;
