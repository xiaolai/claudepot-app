import Link from "next/link";
import { notFound } from "next/navigation";
import { Cpu, ExternalLink, ArrowLeft } from "lucide-react";
import {
  getDecisionsBySubmission,
  getOfficeDecisionById,
} from "@/db/office-queries";
import { OFFICE_PERSONAS } from "@/lib/office-personas";
import { relativeTime } from "@/lib/format";

export const dynamic = "force-dynamic";

const VERDICT_PHRASE: Record<string, string> = {
  feed:        "Accepted onto the feed.",
  human_queue: "Borderline — sent to human review.",
  firehose:    "Not accepted onto the main feed.",
};

function humanCriterion(id: string): string {
  return id.replace(/_/g, " ");
}

export default async function DecisionPage({
  params,
}: {
  params: Promise<{ id: string }>;
}) {
  const { id } = await params;
  const d = await getOfficeDecisionById(id);
  if (!d) notFound();

  const persona = OFFICE_PERSONAS[d.appliedPersona];
  const failedGates = Object.entries(d.inclusionGates)
    .filter(([, v]) => !v)
    .map(([k]) => k);

  // Sibling decisions on the same submission, ordered scoredAt asc
  // per the office's 2026-05-08 ask. Excluded the current decision
  // since it's already the page's primary subject.
  const siblings = (await getDecisionsBySubmission(d.submissionId)).filter(
    (s) => s.id !== d.id,
  );

  const host = d.submissionUrl
    ? new URL(d.submissionUrl).hostname.replace(/^www\./, "")
    : null;

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
          scored by{" "}
          <Link href={`/office/persona/${d.appliedPersona}`}>
            {persona?.display ?? d.appliedPersona}
          </Link>
          {" · "}
          <time dateTime={d.scoredAt.toISOString()}>
            {relativeTime(d.scoredAt.toISOString())}
          </time>
          <span className="office-ai-chip" aria-label="AI-authored decision">
            <Cpu size={10} aria-hidden /> AI
          </span>
        </p>
        <h1 className="office-decision-h1">
          {d.submissionUrl ? (
            <a href={d.submissionUrl} target="_blank" rel="noreferrer noopener">
              {d.submissionTitle}
              {host ? <span className="office-pick-host"> ({host})</span> : null}
              <span className="office-decision-h1-icon" aria-hidden>
                <ExternalLink size={14} />
              </span>
            </a>
          ) : (
            d.submissionTitle
          )}
        </h1>
        <p className="proto-dek office-decision-why">{d.oneLineWhy}</p>
      </header>

      <section className="proto-section">
        <p className={`office-verdict office-routing-${d.routing}`}>
          {VERDICT_PHRASE[d.routing] ?? d.routing}
        </p>
      </section>

      {d.latestOverride && (
        <section className="proto-section">
          <h2>
            Override applied
            {d.latestOverride.reviewerKind === "bot"
              ? " (bot review)"
              : " (human review)"}
          </h2>
          <p className={`office-verdict office-routing-${d.latestOverride.overrideRouting}`}>
            {VERDICT_PHRASE[d.latestOverride.overrideRouting] ??
              d.latestOverride.overrideRouting}
          </p>
          <p className="office-section-lede">{d.latestOverride.reason}</p>
          <p className="office-fineprint">
            Filed{" "}
            <time dateTime={d.latestOverride.createdAt.toISOString()}>
              {relativeTime(d.latestOverride.createdAt.toISOString())}
            </time>
            . The original verdict above is preserved as part of the
            audit trail.
          </p>
        </section>
      )}

      {d.hardRejectsHit.length > 0 && (
        <section className="proto-section">
          <h2>Rules that fired</h2>
          <p className="office-section-lede">
            Hard rejects auto-decline a pick regardless of score. Each is
            spelled out at <Link href="/office/rubric">/office/rubric</Link>.
          </p>
          <ul className="office-list">
            {d.hardRejectsHit.map((id) => (
              <li key={id}><code>{id}</code></li>
            ))}
          </ul>
        </section>
      )}

      {failedGates.length > 0 && (
        <section className="proto-section">
          <h2>Checks that didn&rsquo;t pass</h2>
          <p className="office-section-lede">
            All inclusion gates must pass for a pick to be considered.
          </p>
          <ul className="office-list">
            {failedGates.map((id) => (
              <li key={id}><code>{id}</code></li>
            ))}
          </ul>
        </section>
      )}

      <section className="proto-section">
        <h2>How {persona?.display ?? d.appliedPersona} scored it</h2>
        <p className="office-section-lede">
          Eight criteria from the rubric. Click a criterion name on{" "}
          <Link href="/office/rubric">/office/rubric</Link> to see what each
          score level means.
        </p>
        <table className="office-friendly-scores">
          <tbody>
            {Object.entries(d.perCriterionScores).map(([c, s]) => (
              <tr key={c}>
                <th scope="row">{humanCriterion(c)}</th>
                <td className="office-score-cell">
                  <span className="office-score-value">{s}</span>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
        <p className="office-fineprint">
          Numeric weights and persona multipliers stay private — adversaries
          could optimize against the math. The total is shaped by{" "}
          {persona?.display ?? d.appliedPersona}&rsquo;s overlay; their stance
          is on their <Link href={`/office/persona/${d.appliedPersona}`}>profile</Link>.
        </p>
      </section>

      {siblings.length > 0 && (
        <section className="proto-section">
          <h2>Other decisions on this submission</h2>
          <p className="office-section-lede">
            The office writes more than one decision per submission in
            normal operation — different personas score the same
            piece, and dissenting takes are recorded as their own
            rows.{" "}
            <Link href={`/office/submission/${d.submissionId}`}>
              See the full timeline
            </Link>
            .
          </p>
          <ul className="office-list">
            {siblings.map((s) => {
              const sp = OFFICE_PERSONAS[s.appliedPersona];
              const effRouting = s.latestOverride?.overrideRouting ?? s.routing;
              return (
                <li key={s.id}>
                  <Link href={`/office/decision/${s.id}`}>
                    {sp?.display ?? s.appliedPersona}
                  </Link>
                  {" — "}
                  <span className={`office-routing-${effRouting}`}>
                    {VERDICT_PHRASE[effRouting] ?? effRouting}
                  </span>
                  {" · "}
                  <time dateTime={s.scoredAt.toISOString()}>
                    {relativeTime(s.scoredAt.toISOString())}
                  </time>
                </li>
              );
            })}
          </ul>
        </section>
      )}

      <footer className="proto-section office-decision-provenance">
        <p>
          Rubric v{d.rubricVersion} · audience v{d.audienceDocVersion} ·
          confidence {d.confidence} ·{" "}
          <time dateTime={d.scoredAt.toISOString()}>
            {d.scoredAt.toISOString().slice(0, 16).replace("T", " ")} UTC
          </time>
        </p>
      </footer>
    </div>
  );
}
