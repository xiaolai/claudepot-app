import Link from "next/link";
import { Cpu } from "lucide-react";
import { readPublicRubricView } from "@/lib/editorial-spec";
import { OfficeSidebar } from "@/components/prototype/OfficeSidebar";

export const dynamic = "force-static";

export default function RubricPage() {
  const r = readPublicRubricView();
  return (
    <div className="proto-page-aside">
      <OfficeSidebar current="rubric" />
      <div className="proto-page-aside-content">
      <header className="proto-section">
        <div className="office-eyebrow">
          <Cpu size={14} aria-hidden /> rubric · v{r.version}
        </div>
        <h1>The taste rubric</h1>
        <p className="proto-dek">
          What every agent applies before a link reaches the feed. Per the
          transparency policy, criterion <em>names</em> and <em>descriptions</em>{" "}
          live here; weights, thresholds, and persona multipliers stay private
          so adversaries can&rsquo;t optimize against the math.
        </p>
      </header>

      <section className="proto-section">
        <h2>values</h2>
        <p className="office-fineprint">
          The trade-offs the rubric is making, in three lines.
        </p>
        <dl className="office-dl">
          {Object.entries(r.values).map(([k, v]) => (
            <div key={k}>
              <dt><code>{k}</code></dt>
              <dd>{v}</dd>
            </div>
          ))}
        </dl>
      </section>

      <section className="proto-section">
        <h2>hard rejects</h2>
        <p className="office-fineprint">
          Auto-kill regardless of quality score. The negative space defines
          taste.
        </p>
        <dl className="office-dl">
          {r.hard_rejects.map((hr) => (
            <div key={hr.id}>
              <dt><code>{hr.id}</code></dt>
              <dd>{hr.why}</dd>
            </div>
          ))}
        </dl>
      </section>

      <section className="proto-section">
        <h2>inclusion gates</h2>
        <p className="office-fineprint">
          Must pass <em>all</em> before scoring runs. Cheap, binary checks.
        </p>
        <dl className="office-dl">
          {r.inclusion_gates.map((g) => (
            <div key={g.id}>
              <dt><code>{g.id}</code></dt>
              <dd>{g.check}</dd>
            </div>
          ))}
        </dl>
      </section>

      <section className="proto-section">
        <h2>quality criteria</h2>
        <p className="office-fineprint">
          Eight criteria. Weights are intentionally not shown — see the
          transparency policy for why.
        </p>
        <div className="office-criteria">
          {r.quality_criteria.map((c) => (
            <article key={c.id} className="office-criterion">
              <h3><code>{c.id}</code></h3>
              <pre className="office-criterion-rubric">{c.rubric.trim()}</pre>
            </article>
          ))}
        </div>
      </section>

      <section className="proto-section">
        <h2>recency windows</h2>
        <p className="office-fineprint">
          Per submission-type max age. Beyond the window, the
          <code> within_recency_window </code> gate fails.
        </p>
        <table className="office-scores-table">
          <thead>
            <tr>
              <th>type</th>
              <th>max age (days)</th>
            </tr>
          </thead>
          <tbody>
            {Object.entries(r.recency_windows).map(([type, days]) => (
              <tr key={type}>
                <td><code>{type}</code></td>
                <td>{days}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </section>

      <section className="proto-section">
        <h2>routing destinations</h2>
        <p className="office-fineprint">
          Three destinations after scoring. The thresholds that route between
          them are private.
        </p>
        <dl className="office-dl">
          {Object.entries(r.routing_destinations).map(([k, v]) => (
            <div key={k}>
              <dt><code>{k}</code></dt>
              <dd>{v}</dd>
            </div>
          ))}
        </dl>
      </section>

      <section className="proto-section">
        <h2>format extensions</h2>
        <p className="office-fineprint">
          Additional fields the agent looks for per submission type.
        </p>
        {Object.entries(r.format_extensions).map(([type, fields]) => (
          <div key={type} className="office-format-extension">
            <h3><code>{type}</code></h3>
            <ul className="office-list">
              {fields.map((f) => (
                <li key={f}><code>{f}</code></li>
              ))}
            </ul>
          </div>
        ))}
      </section>

      <section className="proto-section">
        <h2>personas</h2>
        <p className="office-fineprint">
          Editorial agents that score independently. Each carries a stance;
          the multipliers that bias each agent toward its specialty are
          private.
        </p>
        <dl className="office-dl">
          {r.persona_descriptions.map((p) => (
            <div key={p.id}>
              <dt>
                <Link href={`/office/persona/${p.id}`}><code>{p.id}</code></Link>
              </dt>
              <dd>{p.description}</dd>
            </div>
          ))}
        </dl>
      </section>
      </div>
    </div>
  );
}
