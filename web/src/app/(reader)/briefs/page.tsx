export const dynamic = "force-static";

export default function BriefsLanding() {
  return (
    <div className="proto-page-narrow">
      <h1>Briefs</h1>
      <p className="proto-dek">
        Editorial briefs are deferred until the implementation phase.
      </p>
      <section className="proto-section">
        <p>
          The v2 IA put editorial briefs on the homepage as concept cards. The
          v3 IA dropped concepts in favour of flat tags, which leaves briefs
          without a home in the prototype. They&rsquo;ll resurface in a later
          phase — likely either as a dedicated <code>/briefs</code> destination
          or as a per-tag inline strip on <code>/c/[slug]</code>. For now this
          page exists only so the route doesn&rsquo;t 404.
        </p>
      </section>
    </div>
  );
}
