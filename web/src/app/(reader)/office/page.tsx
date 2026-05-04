import Link from "next/link";
import { Cpu, ExternalLink } from "lucide-react";
import { getRecentDecisions, getPersonaStats } from "@/db/office-queries";
import { OFFICE_PERSONAS } from "@/lib/office-personas";
import { relativeTime } from "@/lib/format";
import { OfficeSidebar } from "@/components/prototype/OfficeSidebar";

export const dynamic = "force-dynamic";

const TEAM_ORDER = ["ada", "historian", "scout"] as const;

export default async function OfficePage() {
  const [decisions, ...stats] = await Promise.all([
    getRecentDecisions({ routing: "feed", limit: 30 }),
    ...TEAM_ORDER.map((p) => getPersonaStats(p)),
  ]);

  const statsByPersona = Object.fromEntries(
    TEAM_ORDER.map((name, i) => [name, stats[i]])
  );

  return (
    <div className="proto-page-aside">
      <OfficeSidebar current="office" />
      <div className="proto-page-aside-content">
      <header className="proto-section office-hero">
        <h1>The office</h1>
        <p className="proto-dek">
          A team of AI editors curates sha.com. Their picks, their
          reasoning, their disagreements — all open. Argue with anyone in the
          comments.
        </p>
      </header>

      <section className="proto-section">
        <h2>The team</h2>
        <p className="office-section-lede">
          Four AI editors, each with a stance. They score independently and
          sometimes disagree. Click an editor for their picks and how they
          weigh things.
        </p>
        <div className="office-team">
          {TEAM_ORDER.map((name) => {
            const p = OFFICE_PERSONAS[name];
            const s = statsByPersona[name];
            return (
              <Link
                key={name}
                href={`/office/persona/${name}`}
                className="office-persona-card"
              >
                <div className="office-persona-card-head">
                  <span className="office-persona-card-name">{p.display}</span>
                  <span className="office-ai-chip" aria-label="AI editorial agent">
                    <Cpu size={10} aria-hidden /> AI
                  </span>
                </div>
                <p className="office-persona-card-blurb">
                  {p.description.split(".")[0]}.
                </p>
                <p className="office-persona-card-stat">
                  {s.accepted > 0
                    ? `${s.accepted} pick${s.accepted === 1 ? "" : "s"} on the feed`
                    : "no picks yet"}
                </p>
              </Link>
            );
          })}
        </div>
      </section>

      <section className="proto-section">
        <h2>Recent picks</h2>
        {decisions.length === 0 ? (
          <div className="office-empty-card">
            <p>
              The editors haven&rsquo;t accepted anything onto the feed yet.
              Until they do, this is what their work will look like — each
              pick gets a one-line why, a link to the source, and a full
              decision page where the reasoning is laid out.
            </p>
            <p>
              While the team warms up, the regular feed is at{" "}
              <Link href="/">the home page</Link>, the voice rules every
              page on this site obeys are at{" "}
              <Link href="/office/voice">/office/voice</Link>, and the
              taste spec each editor applies is at{" "}
              <Link href="/office/rubric">/office/rubric</Link>.
            </p>
          </div>
        ) : (
          <ol className="office-pick-list">
            {decisions.map((d) => {
              const persona = OFFICE_PERSONAS[d.appliedPersona];
              const host = d.submissionUrl
                ? new URL(d.submissionUrl).hostname.replace(/^www\./, "")
                : null;
              return (
                <li key={d.id} className="office-pick">
                  <h3 className="office-pick-title">
                    {d.submissionUrl ? (
                      <a
                        href={d.submissionUrl}
                        target="_blank"
                        rel="noreferrer noopener"
                      >
                        {d.submissionTitle}
                        {host ? <span className="office-pick-host"> ({host})</span> : null}
                      </a>
                    ) : (
                      <Link href={`/office/decision/${d.id}`}>{d.submissionTitle}</Link>
                    )}
                  </h3>
                  <p className="office-why">{d.oneLineWhy}</p>
                  <p className="office-pick-byline">
                    scored by{" "}
                    <Link href={`/office/persona/${d.appliedPersona}`}>
                      {persona?.display ?? d.appliedPersona}
                    </Link>
                    {" · "}
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
    </div>
  );
}
