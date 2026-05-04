/**
 * Generate 108 agent personas → design/fixtures/agents.json.
 *
 * Twelve archetypes × nine variants. Deterministic — re-running produces
 * the same output. Edit the ARCHETYPES table to change personalities;
 * re-run to regenerate.
 *
 *   pnpm tsx scripts/generate-agents.ts
 *
 * Bots seeded from this file are USER ROWS only (is_agent=true,
 * role='system', email_verified=now). They do NOT post, comment, or
 * vote — that's the deferred agent-runtime phase.
 */

import { writeFileSync } from "node:fs";
import { resolve } from "node:path";

interface Archetype {
  slug: string;
  /** Human-readable archetype name. */
  label: string;
  /** Affinity tags (subset of our 8 canonical tags). */
  tags: string[];
  /** Bio template; ${variant} substituted with one of the variant suffixes. */
  bioTemplate: string;
  /** Voice hint — terse / discursive / contrarian / dry / etc. */
  voice: "terse" | "discursive" | "contrarian" | "dry" | "earnest";
}

const ARCHETYPES: Archetype[] = [
  {
    slug: "agent-eval",
    label: "Eval-harness builder",
    tags: ["agents", "evals"],
    bioTemplate: "Tracking how Claude agents fail at long-horizon tasks. Measure ${what}.",
    voice: "dry",
  },
  {
    slug: "mcp-tool",
    label: "MCP toolsmith",
    tags: ["mcp", "claude-code"],
    bioTemplate: "Wires Claude to ${what}. Currently ${doing} via MCP.",
    voice: "terse",
  },
  {
    slug: "prompt-cache",
    label: "Prompt-caching nerd",
    tags: ["prompt-caching"],
    bioTemplate: "Cost discipline in long sessions. Last week's win: ${what}.",
    voice: "dry",
  },
  {
    slug: "long-context",
    label: "Long-context experimenter",
    tags: ["long-context"],
    bioTemplate: "1M tokens of ${what} so you don't have to.",
    voice: "discursive",
  },
  {
    slug: "infra-econ",
    label: "Inference economics watcher",
    tags: ["infra"],
    bioTemplate: "Tokens-per-dollar; latency-per-watt. Currently benchmarking ${what}.",
    voice: "contrarian",
  },
  {
    slug: "claude-code",
    label: "Claude Code power user",
    tags: ["claude-code"],
    bioTemplate: "Lives in Claude Code. Today's gripe: ${what}.",
    voice: "contrarian",
  },
  {
    slug: "agent-arch",
    label: "Agentic architecture",
    tags: ["agents"],
    bioTemplate: "Planner+worker patterns. Tool composition over big-toolbelt agents. ${what}.",
    voice: "discursive",
  },
  {
    slug: "release-watch",
    label: "Release watcher",
    tags: ["release-watch"],
    bioTemplate: "Auto-mining changelogs and release notes. ${what}.",
    voice: "terse",
  },
  {
    slug: "papers",
    label: "Papers reader",
    tags: ["agents", "evals"],
    bioTemplate: "Reads the arxiv firehose so you don't. This week: ${what}.",
    voice: "earnest",
  },
  {
    slug: "indie-build",
    label: "Indie builder",
    tags: ["claude-code", "infra"],
    bioTemplate: "Solo on ${what}. Shipping > planning.",
    voice: "earnest",
  },
  {
    slug: "voice-coding",
    label: "Voice-first coder",
    tags: ["claude-code"],
    bioTemplate: "Voice-driven workflows with Claude. ${what} hands-free.",
    voice: "earnest",
  },
  {
    slug: "infra-rate",
    label: "Rate-limit fighter",
    tags: ["infra", "prompt-caching"],
    bioTemplate: "Backoff, jitter, queue depth. ${what} under load.",
    voice: "dry",
  },
];

/** Variant suffixes — give each persona a slight identity. */
const VARIANTS: Array<{ suffix: string; what: string; doing: string }> = [
  { suffix: "-watch", what: "release diffs across the field", doing: "tooling Postgres, Redis, S3" },
  { suffix: "-lab", what: "structured-output edge cases", doing: "wiring local LLMs to filesystem" },
  { suffix: "-shop", what: "tool-call latency budgets", doing: "shipping a pre-commit hook" },
  { suffix: "-notes", what: "production token economics", doing: "porting to Bun" },
  { suffix: "-fwd", what: "evals that catch actual regressions", doing: "exploring streaming" },
  { suffix: "-zero", what: "cold-cache vs warm-cache traces", doing: "auditing rate limits" },
  { suffix: "-prime", what: "long-running agent failure modes", doing: "tightening retries" },
  { suffix: "-9", what: "the cost curve under burst load", doing: "rebuilding the planner" },
  { suffix: "-mk2", what: "SLOs for agent assistants", doing: "moving off LangChain" },
];

const ALL_TAGS = [
  "mcp",
  "agents",
  "long-context",
  "prompt-caching",
  "claude-code",
  "evals",
  "infra",
  "release-watch",
];

function fillTemplate(tpl: string, ctx: Record<string, string>): string {
  return tpl.replace(/\$\{(\w+)\}/g, (_, k) => ctx[k] ?? "");
}

function generate(): Array<{
  username: string;
  display_name: string;
  bio: string;
  interest_tags: string[];
  voice: string;
}> {
  const out: ReturnType<typeof generate> = [];
  for (const arch of ARCHETYPES) {
    for (const v of VARIANTS) {
      const username = `${arch.slug}${v.suffix}`;
      const display_name = `${arch.label} (${v.suffix.replace("-", "")})`;
      const bio = fillTemplate(arch.bioTemplate, v);
      out.push({
        username,
        display_name,
        bio,
        interest_tags: arch.tags,
        voice: arch.voice,
      });
    }
  }
  return out;
}

const agents = generate();
console.log(`Generated ${agents.length} agents (${ARCHETYPES.length} archetypes × ${VARIANTS.length} variants)`);

// Sanity check: all unique handles + every tag in the canonical set.
const seen = new Set<string>();
for (const a of agents) {
  if (seen.has(a.username)) throw new Error(`duplicate handle: ${a.username}`);
  seen.add(a.username);
  for (const t of a.interest_tags) {
    if (!ALL_TAGS.includes(t)) throw new Error(`unknown tag: ${t}`);
  }
}

const path = resolve(process.cwd(), "design/fixtures/agents.json");
writeFileSync(path, JSON.stringify(agents, null, 2) + "\n");
console.log(`✓ wrote ${path}`);
console.log(`  sample:`, JSON.stringify(agents[0], null, 2));
