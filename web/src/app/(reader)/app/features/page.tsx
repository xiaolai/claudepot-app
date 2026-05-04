import Link from "next/link";
import type { Metadata } from "next";

export const metadata: Metadata = {
  title: "Features",
  description:
    "Eight tabs in ClauDepot, mapping one-to-one to the Tauri app's primary navigation.",
};

const FEATURES = [
  {
    slug: "accounts",
    title: "Accounts",
    summary:
      "Add, switch, and verify Anthropic accounts. CLI and Desktop slots are independent.",
  },
  {
    slug: "activities",
    title: "Activities",
    summary:
      "Live view of every running Claude session, sorted by who needs your attention first. Plus today/month dashboard and an event stream.",
  },
  {
    slug: "projects",
    title: "Projects",
    summary:
      "Per-project sessions in a master-detail pane. Rename safely (journaled, reversible), move, search transcripts.",
  },
  {
    slug: "keys",
    title: "Keys",
    summary:
      "Manage API keys for third-party services. Self-clearing clipboard, never echoed, never logged.",
  },
  {
    slug: "third-parties",
    title: "Third-parties",
    summary:
      "Configure tools that integrate with Claude — GitHub tokens, Linear, etc. — without leaking secrets.",
  },
  {
    slug: "automations",
    title: "Automations",
    summary:
      "Cron-scheduled prompts. Run a prompt every weekday morning, every Monday, or any cron schedule.",
  },
  {
    slug: "global",
    title: "Global",
    summary:
      "Per-machine config: paths, defaults, behaviors that apply across all accounts.",
  },
  {
    slug: "settings",
    title: "Settings",
    summary:
      "Themes, shortcuts, notifications. Cleanup (session prune + slim + trash with 7-day undo) lives here.",
  },
];

export default function FeaturesIndex() {
  return (
    <article>
      <h1>Features</h1>
      <p>
        ClauDepot is organised into eight tabs that mirror the Tauri app&rsquo;s
        primary navigation. Each tab is self-contained &mdash; you can use any
        one of them without configuring the others.
      </p>
      <div className="docs-feature-grid">
        {FEATURES.map((f) => (
          <Link
            key={f.slug}
            href={`/app/features/${f.slug}`}
            className="docs-feature-card"
          >
            <h3>{f.title}</h3>
            <p>{f.summary}</p>
          </Link>
        ))}
      </div>
    </article>
  );
}
