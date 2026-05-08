/**
 * /office/submission/[id]/ — every decision the office made on a
 * single submission, ordered chronologically. Per the office's
 * 2026-05-08 polity API ask: "the schema already permits multiple
 * decision_records per submission. Office will write more than
 * one in normal operation. Asking that the page render all of
 * them in a stable order (suggest: scoredAt asc), with a per-
 * criterion view that shows where decisions agreed vs diverged
 * on the jsonb scores."
 *
 * Privacy: per editorial/transparency.md the per-criterion scores
 * are public; weights and persona multipliers stay private. The
 * comparison view here only shows scores — never weighted
 * contributions or thresholds.
 */

import Link from "next/link";
import { notFound } from "next/navigation";
import { ArrowLeft, Cpu, ExternalLink } from "lucide-react";
import { eq } from "drizzle-orm";

import { db } from "@/db/client";
import { submissions, users } from "@/db/schema";
import { getDecisionsBySubmission } from "@/db/office-queries";
import { OFFICE_PERSONAS } from "@/lib/office-personas";
import { relativeTime } from "@/lib/format";

export const dynamic = "force-dynamic";

const VERDICT_PHRASE: Record<string, string> = {
  feed: "Accepted onto the feed.",
  human_queue: "Borderline — sent to human review.",
  firehose: "Not accepted onto the main feed.",
};

function humanCriterion(id: string): string {
  return id.replace(/_/g, " ");
}

export default async function SubmissionDecisionsPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = await params;

  const [sub] = await db
    .select({
      id: submissions.id,
      title: submissions.title,
      url: submissions.url,
      authorUsername: users.username,
    })
    .from(submissions)
    .innerJoin(users, eq(users.id, submissions.authorId))
    .where(eq(submissions.id, id))
    .limit(1);
  if (!sub) notFound();

  const decisions = await getDecisionsBySubmission(id);
  if (decisions.length === 0) notFound();

  const host = sub.url
    ? new URL(sub.url).hostname.replace(/^www\./, "")
    : null;

  // Comparison view: union of all criterion keys across decisions
  // (the office may add criteria over time without a polity
  // migration), each row showing one persona's score per cell.
  // Stable column order = chronological (decisions array is already
  // scoredAt asc). Stable row order = first-seen criterion order
  // across decisions.
  const criterionOrder: string[] = [];
  const seen = new Set<string>();
  for (const d of decisions) {
    for (const k of Object.keys(d.perCriterionScores)) {
      if (!seen.has(k)) {
        seen.add(k);
        criterionOrder.push(k);
      }
    }
  }

  return (
    <div className="proto-page-narrow">
      <nav className="office-breadcrumb">
        <Link href="/office">
          <span className="proto-inline-icon" aria-hidden>
            <ArrowLeft size={12} />
          </span>{" "}
          the office
        </Link>
      </nav>

      <header className="proto-section">
        <p className="office-pick-byline">
          {decisions.length === 1
            ? "1 editorial decision"
            : `${decisions.length} editorial decisions`}
          {" · oldest first"}
          <span className="office-ai-chip" aria-label="AI-authored decisions">
            <Cpu size={10} aria-hidden /> AI
          </span>
        </p>
        <h1 className="office-decision-h1">
          {sub.url ? (
            <a href={sub.url} target="_blank" rel="noreferrer noopener">
              {sub.title}
              {host ? <span className="office-pick-host"> ({host})</span> : null}
              <span className="office-decision-h1-icon" aria-hidden>
                <ExternalLink size={14} />
              </span>
            </a>
          ) : (
            sub.title
          )}
        </h1>
      </header>

      <section className="proto-section">
        <h2>Timeline</h2>
        <ul className="office-list">
          {decisions.map((d) => {
            const persona = OFFICE_PERSONAS[d.appliedPersona];
            const effectiveRouting = d.latestOverride?.overrideRouting ?? d.routing;
            const wasOverridden =
              d.latestOverride != null && effectiveRouting !== d.routing;
            return (
              <li key={d.id}>
                <Link href={`/office/decision/${d.id}`}>
                  {persona?.display ?? d.appliedPersona}
                </Link>
                {" — "}
                <span className={`office-routing-${effectiveRouting}`}>
                  {VERDICT_PHRASE[effectiveRouting] ?? effectiveRouting}
                </span>
                {wasOverridden && (
                  <>
                    {" "}
                    <span className="office-fineprint">
                      (overridden{" "}
                      {d.latestOverride?.reviewerKind === "bot"
                        ? "by bot"
                        : "by staff"}
                      , originally{" "}
                      {VERDICT_PHRASE[d.routing] ?? d.routing})
                    </span>
                  </>
                )}
                {" · "}
                <time dateTime={d.scoredAt.toISOString()}>
                  {relativeTime(d.scoredAt.toISOString())}
                </time>
                <p className="proto-dek office-decision-why">{d.oneLineWhy}</p>
              </li>
            );
          })}
        </ul>
      </section>

      {decisions.length > 1 && criterionOrder.length > 0 && (
        <section className="proto-section">
          <h2>Per-criterion comparison</h2>
          <p className="office-section-lede">
            Where the personas agreed and where they diverged. Click
            a criterion name on{" "}
            <Link href="/office/rubric">/office/rubric</Link> for the
            full rubric. Numeric weights and persona multipliers stay
            private — adversaries could optimize against the math.
          </p>
          <table className="office-friendly-scores">
            <thead>
              <tr>
                <th scope="col">Criterion</th>
                {decisions.map((d) => {
                  const persona = OFFICE_PERSONAS[d.appliedPersona];
                  return (
                    <th key={d.id} scope="col">
                      <Link href={`/office/decision/${d.id}`}>
                        {persona?.display ?? d.appliedPersona}
                      </Link>
                    </th>
                  );
                })}
              </tr>
            </thead>
            <tbody>
              {criterionOrder.map((c) => (
                <tr key={c}>
                  <th scope="row">{humanCriterion(c)}</th>
                  {decisions.map((d) => {
                    const v = d.perCriterionScores[c];
                    return (
                      <td key={d.id} className="office-score-cell">
                        <span className="office-score-value">
                          {v == null ? "—" : v}
                        </span>
                      </td>
                    );
                  })}
                </tr>
              ))}
            </tbody>
          </table>
        </section>
      )}
    </div>
  );
}
