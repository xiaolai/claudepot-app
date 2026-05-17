import Link from "next/link";
import type { Metadata } from "next";

export const metadata: Metadata = {
  title: "Features",
  description:
    "Nine tabs in ClauDepot, mapping one-to-one to the Tauri app's primary navigation.",
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
      "Per-project sessions in a master-detail pane. Rename safely (journaled, reversible), move, search transcripts. Time-boxed permission grants and per-project `.env` editing live here too.",
  },
  {
    slug: "memory",
    title: "Memory",
    summary:
      "Cross-harness shared memory and indexed transcript search across Claude Code and Codex. Durable memories, decisions, and evidence with an MCP server so a running Claude session can query the same store.",
  },
  {
    slug: "keys",
    title: "Keys",
    summary:
      "API keys for third-party services plus a local secret vault — copy or inject into a project's `.env` without ever rendering the value in the DOM.",
  },
  {
    slug: "third-parties",
    title: "Third-parties",
    summary:
      "Run non-Anthropic models through the same `claude` interface. Wrapper binaries on PATH; first-party Claude is never touched.",
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
      "Read-only inspection of Claude Code's machine-wide state: config layers, CLAUDE.md health, the tips ledger, updates for both apps.",
  },
  {
    slug: "settings",
    title: "Settings",
    summary:
      "Thirteen sub-panes: prefs, appearance, notifications, network, auto-rotation rules, health, MCP installer, cleanup (prune + slim + trash with 7-day undo), protected paths, GitHub PAT, locks, diagnostics, About.",
  },
];

export default function FeaturesIndex() {
  return (
    <article>
      <h1>Features</h1>
      <p>
        ClauDepot is organised into nine tabs that mirror the Tauri app&rsquo;s
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
