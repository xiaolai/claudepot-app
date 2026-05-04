import Link from "next/link";
import type { Metadata } from "next";

export const metadata: Metadata = {
  title: "ClauDepot — control panel for Claude Code and Claude Desktop",
  description:
    "Switch accounts. Watch what's running. Schedule prompts. Find old chats. Reclaim disk space.",
};

const HIGHLIGHTS = [
  {
    title: "One click between accounts",
    body:
      "Keep work and personal accounts side by side. Switch the CLI and Desktop slots independently.",
    href: "/app/features/accounts",
  },
  {
    title: "See what Claude is doing right now",
    body:
      "A live view of every running session, sorted by who needs your attention first.",
    href: "/app/features/activities",
  },
  {
    title: "Find any old chat",
    body:
      "Cross-project search across every transcript Claude has ever written. Reopen, export, share.",
    href: "/app/features/projects",
  },
];

export default function AppLanding() {
  return (
    <article>
      <section className="docs-hero">
        <h1>A control panel for Claude.</h1>
        <p className="docs-hero-tagline">
          Switch accounts. Watch what&rsquo;s running. Schedule prompts. Find
          old chats. Reclaim disk space.
        </p>
        <div className="docs-hero-actions">
          <Link href="/app/install" className="docs-cta-primary">
            Get started
          </Link>
          <Link href="/app/why" className="docs-cta-secondary">
            Why ClauDepot?
          </Link>
        </div>
      </section>

      <section>
        <h2>What it does</h2>
        <div className="docs-feature-grid">
          {HIGHLIGHTS.map((h) => (
            <Link key={h.href} href={h.href} className="docs-feature-card">
              <h3>{h.title}</h3>
              <p>{h.body}</p>
            </Link>
          ))}
        </div>
        <p>
          See <Link href="/app/features">all features</Link> for the full list:
          accounts, activities, projects, keys, third-parties, automations,
          global, settings.
        </p>
      </section>

      <section>
        <h2>Open source</h2>
        <p>
          ClauDepot is a Tauri 2 + Rust + React desktop app, ISC-licensed, with
          source on{" "}
          <Link href="https://github.com/xiaolai/claudepot-app">GitHub</Link>.
          Releases are signed binaries for macOS (Intel + Apple Silicon),
          Linux, and Windows.
        </p>
      </section>
    </article>
  );
}
