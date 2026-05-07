import Link from "next/link";
import { Cpu, Shield } from "lucide-react";
import { count, gte, sql } from "drizzle-orm";

import { db } from "@/db/client";
import { policyDecisions } from "@/db/schema";
import { POLICY_CATEGORIES } from "@/lib/moderation";
import { getActiveSystemPrompt } from "@/lib/moderation/prompt-store";
import { OfficeSidebar } from "@/components/prototype/OfficeSidebar";

export const dynamic = "force-dynamic";

/**
 * /office/policy — Ada's other job.
 *
 * Public window into the AI policy moderator. Aggregate-only:
 * counts by category over rolling 30 days, the active prompt
 * version, the five-category taxonomy with definitions. Does NOT
 * expose per-decision detail — those stay private to the affected
 * user (via /appeal/[id]) and to staff (via /admin/log).
 *
 * Per dev-docs/policy-moderator-plan.md §13 (privacy + transparency):
 * the policy prompt is public; per-decision rows are private; the
 * aggregate is the right granularity for a public audit surface.
 */

const CATEGORY_NOTES: Record<string, string> = {
  spam: "Promotional content with no surrounding discussion, link farms, repetitive postings, paid promotion without disclosure.",
  abuse: "Harassment, slurs, threats, targeted personal attacks against an identified person or group.",
  illegal:
    "CSAM; distributing malware or stolen credentials; flagrant copyright violation. Discussion of these is allowed; distribution is not.",
  doxxing:
    "Exposing a private individual's home address, phone number, government ID, or non-public personal email tied to a real-name target.",
  // Legacy category — retired in POLICY_PROMPT_V="2". Kept here so
  // historical policy_decisions rows with category='off_topic'
  // render with a recognizable label on this page if they surface
  // in the aggregate. New rejects never use this category.
  off_topic: "[legacy] Submissions retired from this category in v2.",
};

export default async function OfficePolicyPage() {
  const startOfWindow = new Date(Date.now() - 30 * 86_400_000);

  // Total decisions in window + rejects in window + by-category breakdown.
  const [{ total }, { rejected }, byCategoryRaw] = await Promise.all([
    db
      .select({ total: count() })
      .from(policyDecisions)
      .where(gte(policyDecisions.decidedAt, startOfWindow))
      .then((r) => r[0] ?? { total: 0 }),
    db
      .select({ rejected: count() })
      .from(policyDecisions)
      .where(
        sql`${policyDecisions.decidedAt} >= ${startOfWindow} AND ${policyDecisions.verdict} = 'reject'`,
      )
      .then((r) => r[0] ?? { rejected: 0 }),
    db
      .select({
        category: policyDecisions.category,
        n: count(),
      })
      .from(policyDecisions)
      .where(
        sql`${policyDecisions.decidedAt} >= ${startOfWindow} AND ${policyDecisions.verdict} = 'reject'`,
      )
      .groupBy(policyDecisions.category),
  ]);

  const byCategory = new Map<string, number>();
  for (const r of byCategoryRaw) {
    if (r.category) byCategory.set(r.category, r.n);
  }

  const { version: activePromptVersion } = await getActiveSystemPrompt();

  return (
    <div className="proto-page-aside">
      <OfficeSidebar current="policy" />
      <div className="proto-page-aside-content">
      <header className="proto-section">
        <div className="office-persona-head">
          <h1>Policy moderation</h1>
          <span className="office-ai-chip" aria-label="AI policy moderator">
            <Shield size={11} aria-hidden /> Ada&rsquo;s other job
          </span>
        </div>
        <p className="proto-dek">
          Every submission and comment runs through a synchronous AI
          policy gate before publish. The job belongs to{" "}
          <Link href="/office/persona/ada">Ada</Link> — same persona who
          scores editorial picks, here in a different mode. The full
          system prompt is in this repo (
          <Link href="/office/rubric">rubric &amp; voice are public</Link>);
          the moderator&rsquo;s per-decision reasoning stays private to
          the affected user and to staff so the public log doesn&rsquo;t
          re-expose the very PII it just classified.
        </p>
      </header>

      <section className="proto-section">
        <h2>Categories</h2>
        <p className="office-section-lede">
          Universal trust-and-safety taxonomy. Reject only when
          content clearly fits one. When in doubt, pass. Topical
          fit (does this submission match the platform&rsquo;s
          audience) is NOT a moderator concern — that&rsquo;s
          handled by editorial scoring and community voting, not
          by the gate.
        </p>
        <dl className="office-policy-categories">
          {POLICY_CATEGORIES.map((cat) => (
            <div key={cat} className="office-policy-category">
              <dt>
                <code>{cat}</code>{" "}
                {byCategory.has(cat) ? (
                  <span className="office-pick-host">
                    ({byCategory.get(cat)} in last 30d)
                  </span>
                ) : null}
              </dt>
              <dd>{CATEGORY_NOTES[cat]}</dd>
            </div>
          ))}
        </dl>
      </section>

      <section className="proto-section">
        <h2>Last 30 days</h2>
        {total === 0 ? (
          <p className="office-empty">
            Ada hasn&rsquo;t scored anything yet (or the window is
            quiet). Aggregate stats land here as decisions accumulate.
          </p>
        ) : (
          <p className="office-stat-summary">
            <strong>{total}</strong> decision{total === 1 ? "" : "s"};{" "}
            <strong>{rejected}</strong> reject{rejected === 1 ? "" : "s"}{" "}
            ({total > 0 ? Math.round((rejected / total) * 100) : 0}%).
          </p>
        )}
      </section>

      <section className="proto-section">
        <h2>The prompt</h2>
        <p className="proto-dek">
          Active version:{" "}
          <code>
            <Cpu size={10} aria-hidden /> {activePromptVersion}
          </code>
          . Staff edits it at{" "}
          <code>/admin/console/policy</code> when the false-positive
          rate calls for it. Versioned: every saved version is in the
          DB under its label, and{" "}
          <Link href="/admin/log?automated=1">/admin/log</Link>{" "}
          (visible to any signed-in user) shows when the prompt
          changed.
        </p>
        <p className="office-fineprint">
          Why no prompt body here: the policy prompt is{" "}
          <em>public</em> by design (industry-standard categories;
          not adversarially tunable in the way the editorial taste
          rubric is) — but rendering the full text on a public
          aggregate page would make it look like editorial copy.
          It&rsquo;s in <code>web/src/lib/moderation/prompt.ts</code>{" "}
          (the fallback) and in the <code>moderation_prompts</code>{" "}
          table (the active version).
        </p>
      </section>

      <section className="proto-section">
        <h2>What this page does NOT show</h2>
        <ul className="office-fineprint-list">
          <li>
            Per-decision details — the moderator&rsquo;s one-line-why
            on a doxxing reject can quote the very address that
            triggered the rule. Drill-down lives in{" "}
            <code>/admin/log</code> (staff only).
          </li>
          <li>
            Author identities — counts here are per-category, not
            per-user.
          </li>
          <li>
            Appeals — they go to staff via the triage inbox at{" "}
            <code>/admin</code>; resolutions land in{" "}
            <code>/admin/log</code>.
          </li>
        </ul>
      </section>
      </div>
    </div>
  );
}
