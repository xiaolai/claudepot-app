import Link from "next/link";
import { and, count, desc, eq } from "drizzle-orm";

import { db } from "@/db/client";
import { botReports, tags, users } from "@/db/schema";
import { links } from "@/db/schema/links";
import { ProposalActionButton } from "@/components/admin/ProposalActionButton";

interface Props {
  asSuffix: string;
}

/**
 * Callouts above the inbox stream — items that don't fit the
 * time-ordered content feed but still want operator attention.
 *
 *   - Pending vocabulary (Ada-proposed tags awaiting review).
 *   - Open bot proposals — every is_agent's open `proposal` row
 *     surfaces here with inline accept/reject. Same notice-strip
 *     UX as pending vocab; one place for "things that want a
 *     human ack."
 *
 * Renders nothing when both feeds are empty.
 */
export async function NoticeStrip({ asSuffix }: Props) {
  const [pendingTagRow, openProposals, pendingLinkRow] = await Promise.all([
    db
      .select({ n: count() })
      .from(tags)
      .where(eq(tags.pendingReview, true))
      .then((rows) => rows[0] ?? { n: 0 }),
    db
      .select({
        id: botReports.id,
        payload: botReports.payload,
        reportedAt: botReports.reportedAt,
        botUsername: users.username,
      })
      .from(botReports)
      .innerJoin(users, eq(users.id, botReports.botId))
      .where(
        and(eq(botReports.kind, "proposal"), eq(botReports.status, "open")),
      )
      .orderBy(desc(botReports.reportedAt))
      .limit(20),
    db
      .select({ n: count() })
      .from(links)
      .where(eq(links.status, "pending"))
      .then((rows) => rows[0] ?? { n: 0 }),
  ]);

  const pendingTagCount = pendingTagRow.n;
  const pendingLinkCount = pendingLinkRow.n;

  if (
    pendingTagCount === 0 &&
    pendingLinkCount === 0 &&
    openProposals.length === 0
  )
    return null;

  return (
    <ul className="proto-inbox-notices" aria-label="Notices">
      {pendingTagCount > 0 ? (
        <li className="proto-inbox-notice">
          <strong>{pendingTagCount}</strong>{" "}
          tag{pendingTagCount === 1 ? "" : "s"} awaiting vocabulary review.{" "}
          <Link href={`/admin/console/vocabulary${asSuffix}`}>Review →</Link>
        </li>
      ) : null}
      {pendingLinkCount > 0 ? (
        <li className="proto-inbox-notice">
          <strong>{pendingLinkCount}</strong>{" "}
          link{pendingLinkCount === 1 ? "" : "s"} suggested for the directory.{" "}
          <Link href={`/admin/links${asSuffix}`}>Review →</Link>
        </li>
      ) : null}
      {openProposals.map((p) => {
        const payload = p.payload as Record<string, unknown>;
        const subKind =
          typeof payload.kind === "string" ? payload.kind : "general";
        const reason =
          typeof payload.reason === "string" ? payload.reason : "";
        const target =
          typeof payload.target === "string" ? payload.target : null;
        return (
          <li key={p.id} className="proto-inbox-notice proto-inbox-notice-proposal">
            <div className="proto-inbox-notice-body">
              <span className="proto-state-pill proto-state-pill-pending">
                {subKind}
              </span>{" "}
              <Link href={`/admin/console/bots/${p.botUsername}${asSuffix}`}>
                @{p.botUsername}
              </Link>{" "}
              proposes: <em>{reason}</em>
              {target ? (
                <>
                  {" "}
                  <span className="proto-meta-quiet">
                    target: <code>{target}</code>
                  </span>
                </>
              ) : null}
            </div>
            <div className="proto-inbox-notice-actions">
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
  );
}
