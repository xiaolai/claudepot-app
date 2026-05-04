import Link from "next/link";
import { notFound } from "next/navigation";
import type React from "react";
import { Cpu, ExternalLink, ArrowLeft } from "lucide-react";
import { getRecentDecisions, getPersonaStats } from "@/db/office-queries";
import { OFFICE_PERSONAS } from "@/lib/office-personas";
import { relativeTime } from "@/lib/format";

export const dynamic = "force-dynamic";

function intersperse(items: string[]): React.ReactNode[] {
  return items.flatMap((item, i) =>
    i === 0 ? [<code key={item}>{item}</code>] : [", ", <code key={item}>{item}</code>]
  );
}

export default async function PersonaPage({
  params,
}: {
  params: Promise<{ name: string }>;
}) {
  const { name } = await params;
  const persona = OFFICE_PERSONAS[name];
  if (!persona) notFound();

  const [stats, recent] = await Promise.all([
    getPersonaStats(name),
    getRecentDecisions({ persona: name, limit: 30 }),
  ]);

  const acceptRate =
    stats.total > 0 ? Math.round((stats.accepted / stats.total) * 100) : null;

  const weighsMore = Object.entries(persona.multipliers).filter(([, m]) => m > 1).map(([c]) => c);
  const weighsLess = Object.entries(persona.multipliers).filter(([, m]) => m < 1).map(([c]) => c);

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
        <div className="office-persona-head">
          <h1>{persona.display}</h1>
          <span className="office-ai-chip" aria-label="AI editorial agent">
            <Cpu size={11} aria-hidden /> AI editor
          </span>
        </div>
        <p className="proto-dek">{persona.description}</p>
      </header>

      {weighsMore.length > 0 && (
        <section className="proto-section">
          <h2>Stance</h2>
          <p className="office-section-lede">
            Editorial agents disagree by design — their multipliers shift
            which criteria matter most. The exact numbers stay private; the
            direction is public.
          </p>
          <p className="office-stance-line">
            <strong>weighs more:</strong> {intersperse(weighsMore)}
          </p>
          {weighsLess.length > 0 && (
            <p className="office-stance-line">
              <strong>weighs less:</strong> {intersperse(weighsLess)}
            </p>
          )}
          <p className="office-fineprint">
            Each criterion is described at{" "}
            <Link href="/office/rubric">/office/rubric</Link>.
          </p>
        </section>
      )}

      <section className="proto-section">
        <h2>Activity</h2>
        {stats.total === 0 ? (
          <p className="office-empty">
            {persona.display} hasn&rsquo;t scored anything yet.
          </p>
        ) : (
          <p className="office-stat-summary">
            {persona.display} has scored <strong>{stats.total}</strong>{" "}
            submission{stats.total === 1 ? "" : "s"};{" "}
            <strong>{acceptRate}%</strong> landed on the feed
            {stats.borderline > 0 ? `, ${stats.borderline} sent to human review` : ""}
            {stats.rejected > 0 ? `, ${stats.rejected} rejected` : ""}.
          </p>
        )}
      </section>

      <section className="proto-section">
        <h2>Recent picks</h2>
        {recent.length === 0 ? (
          <p className="office-empty">
            Nothing yet. Once {persona.display} starts scoring, picks land here.
          </p>
        ) : (
          <ol className="office-pick-list">
            {recent.map((d) => {
              const host = d.submissionUrl
                ? new URL(d.submissionUrl).hostname.replace(/^www\./, "")
                : null;
              return (
                <li key={d.id} className="office-pick">
                  <h3 className="office-pick-title">
                    {d.submissionUrl ? (
                      <a href={d.submissionUrl} target="_blank" rel="noreferrer noopener">
                        {d.submissionTitle}
                        {host ? <span className="office-pick-host"> ({host})</span> : null}
                      </a>
                    ) : (
                      <Link href={`/office/decision/${d.id}`}>{d.submissionTitle}</Link>
                    )}
                  </h3>
                  <p className="office-why">{d.oneLineWhy}</p>
                  <p className="office-pick-byline">
                    <time dateTime={d.scoredAt.toISOString()}>
                      {relativeTime(d.scoredAt.toISOString())}
                    </time>
                    {" · "}
                    <Link href={`/office/decision/${d.id}`} className="office-pick-explain">
                      see reasoning{" "}
                      <span className="proto-inline-icon" aria-hidden>
                        <ExternalLink size={11} />
                      </span>
                    </Link>
                  </p>
                </li>
              );
            })}
          </ol>
        )}
      </section>
    </div>
  );
}
