/**
 * Insert canonical 2025-2026 AI items that were missing from the
 * original seed (the directory was researched in early 2026, so a
 * lot of late-2025 launches and Apr–May 2026 items aren't in it).
 *
 *   pnpm exec tsx --env-file=.env.local scripts/insert-new-links.ts
 *
 * Idempotent via ON CONFLICT (url) DO NOTHING. Slug auto-generated;
 * collisions get a numeric suffix.
 */

import { neon } from "@neondatabase/serverless";

type NewLink = {
  url: string;
  name: string;
  description: string;
  primaryCategorySlug: string;
  categorySlugs?: string[];
};

// Primary category slug → which top-level the entry sits in.
//   "anthropic"       — Anthropic ecosystem (Claude, Claude Code, Skills, SDKs)
//   "mcp"             — MCP servers, registries, clients
//   "coding-tools"    — non-Anthropic IDEs / agent platforms
//   "model-providers" — non-Anthropic frontier model labs
//   "evals"           — leaderboards, benchmarks, eval frameworks
const NEW_LINKS: NewLink[] = [
  // ── Anthropic ecosystem ─────────────────────────────────
  {
    url: "https://www.anthropic.com/engineering/building-agents-with-the-claude-agent-sdk",
    name: "Building agents with the Claude Agent SDK",
    description: "Engineering write-up that became the SDK reference",
    primaryCategorySlug: "anthropic",
    categorySlugs: ["anthropic", "coding-tools"],
  },
  {
    url: "https://www.anthropic.com/engineering/effective-harnesses-for-long-running-agents",
    name: "Effective harnesses for long-running agents",
    description: "Multi-context-window agent design for long-horizon work",
    primaryCategorySlug: "anthropic",
  },
  {
    url: "https://www.anthropic.com/engineering/built-multi-agent-research-system",
    name: "How we built our multi-agent research system",
    description: "Reference reading on parallel agent orchestration",
    primaryCategorySlug: "anthropic",
  },
  {
    url: "https://platform.claude.com/docs/en/agents-and-tools/tool-use/memory-tool",
    name: "Memory tool (docs)",
    description: "Client-side persistent memory tool, beta Sep 2025",
    primaryCategorySlug: "anthropic",
  },
  {
    url: "https://platform.claude.com/docs/en/agents-and-tools/tool-use/code-execution-tool",
    name: "Code execution tool (docs)",
    description: "Sandboxed Python/bash server-side tool",
    primaryCategorySlug: "anthropic",
  },
  {
    url: "https://platform.claude.com/docs/en/agents-and-tools/tool-use/computer-use-tool",
    name: "Computer use tool (docs)",
    description: "Canonical docs for Anthropic's computer-use API",
    primaryCategorySlug: "anthropic",
  },
  {
    url: "https://platform.claude.com/docs/en/build-with-claude/files",
    name: "Files API (docs)",
    description: "Upload-once-use-many file management for the API",
    primaryCategorySlug: "anthropic",
  },
  {
    url: "https://www.anthropic.com/news/introducing-citations-api",
    name: "Citations API",
    description: "Source-grounded responses with passage citations",
    primaryCategorySlug: "anthropic",
  },
  {
    url: "https://www.anthropic.com/news/claude-4",
    name: "Introducing Claude 4",
    description: "Opus 4 + Sonnet 4 family launch (May 2025)",
    primaryCategorySlug: "anthropic",
  },
  {
    url: "https://www.anthropic.com/news/claude-sonnet-4-5",
    name: "Introducing Claude Sonnet 4.5",
    description: "30-hour autonomous run, 77.2% SWE-bench (Sep 2025)",
    primaryCategorySlug: "anthropic",
  },
  {
    url: "https://www.anthropic.com/news/claude-haiku-4-5",
    name: "Introducing Claude Haiku 4.5",
    description: "Sonnet-4-class quality at one-third cost (Oct 2025)",
    primaryCategorySlug: "anthropic",
  },
  {
    url: "https://www.anthropic.com/news/claude-opus-4-5",
    name: "Introducing Claude Opus 4.5",
    description: "Plan Mode + Infinite Chats (Nov 2025)",
    primaryCategorySlug: "anthropic",
  },
  {
    url: "https://www.anthropic.com/news/claude-sonnet-4-6",
    name: "Introducing Claude Sonnet 4.6",
    description: "1M-token-context Sonnet (Feb 2026)",
    primaryCategorySlug: "anthropic",
  },
  {
    url: "https://www.anthropic.com/news/claude-opus-4-6",
    name: "Introducing Claude Opus 4.6",
    description: "1M context, agent teams, Claude in PowerPoint (Feb 2026)",
    primaryCategorySlug: "anthropic",
  },
  {
    url: "https://www.anthropic.com/news/claude-opus-4-7",
    name: "Introducing Claude Opus 4.7",
    description: "Improved long-horizon agentic execution (Apr 2026)",
    primaryCategorySlug: "anthropic",
  },
  {
    url: "https://red.anthropic.com/2026/mythos-preview/",
    name: "Claude Mythos Preview",
    description: "Frontier security model under Project Glasswing (Apr 2026)",
    primaryCategorySlug: "anthropic",
  },
  {
    url: "https://www.anthropic.com/news/claude-code-web",
    name: "Claude Code on the web",
    description: "Browser/iOS Claude Code surface (Oct 2025)",
    primaryCategorySlug: "anthropic",
    categorySlugs: ["anthropic", "coding-tools"],
  },
  {
    url: "https://www.anthropic.com/news/claude-code-plugins",
    name: "Claude Code plugins (announcement)",
    description: "Bundled commands, subagents, MCP, hooks",
    primaryCategorySlug: "anthropic",
    categorySlugs: ["anthropic", "coding-tools"],
  },
  {
    url: "https://www.anthropic.com/product/claude-cowork",
    name: "Claude Cowork",
    description: "Knowledge-work agent in Claude Desktop (GA Apr 2026)",
    primaryCategorySlug: "anthropic",
  },
  {
    url: "https://platform.claude.com/docs/en/managed-agents/overview",
    name: "Claude Managed Agents (docs)",
    description: "Hosted harness for autonomous Claude agents (Apr 2026)",
    primaryCategorySlug: "anthropic",
  },
  {
    url: "https://www.anthropic.com/news/claude-for-chrome",
    name: "Piloting Claude in Chrome",
    description: "Browser-agent extension pilot (Aug 2025)",
    primaryCategorySlug: "anthropic",
  },
  {
    url: "https://alignment.anthropic.com/2025/petri/",
    name: "Petri: open-source auditing tool",
    description: "Automated alignment-evaluation framework (2025)",
    primaryCategorySlug: "anthropic",
    categorySlugs: ["anthropic", "evals"],
  },

  // ── MCP ─────────────────────────────────────────────────
  {
    url: "https://blog.modelcontextprotocol.io/posts/2025-09-08-mcp-registry-preview/",
    name: "MCP Registry preview (announcement)",
    description: "Official open catalog + API for MCP servers",
    primaryCategorySlug: "mcp",
  },
  {
    url: "https://www.anthropic.com/news/donating-the-model-context-protocol-and-establishing-of-the-agentic-ai-foundation",
    name: "Donating MCP to the Agentic AI Foundation",
    description: "Anthropic transfers MCP stewardship (2026)",
    primaryCategorySlug: "mcp",
  },
  {
    url: "https://docs.stripe.com/mcp",
    name: "Stripe MCP (docs)",
    description: "Hosted at mcp.stripe.com with OAuth",
    primaryCategorySlug: "mcp",
  },
  {
    url: "https://developers.notion.com/docs/mcp",
    name: "Notion MCP (developer docs)",
    description: "Hosted OAuth MCP for Notion workspaces",
    primaryCategorySlug: "mcp",
  },
  {
    url: "https://developers.cloudflare.com/agents/model-context-protocol/mcp-servers-for-cloudflare/",
    name: "Cloudflare's MCP servers",
    description: "Catalog of Cloudflare-managed remote MCP servers",
    primaryCategorySlug: "mcp",
  },
  {
    url: "https://docs.sentry.io/product/sentry-mcp/",
    name: "Sentry MCP (docs)",
    description: "Hosted at mcp.sentry.dev with OAuth",
    primaryCategorySlug: "mcp",
  },
  {
    url: "https://docs.slack.dev/ai/slack-mcp-server/",
    name: "Slack MCP server",
    description: "Official remote MCP for Slack content",
    primaryCategorySlug: "mcp",
  },
  {
    url: "https://composio.dev/mcp-gateway",
    name: "Composio MCP Gateway",
    description: "Enterprise-grade unified MCP gateway, 500+ servers",
    primaryCategorySlug: "mcp",
    categorySlugs: ["mcp", "infra"],
  },

  // ── Model providers (non-Anthropic launches) ─────────────
  {
    url: "https://openai.com/index/introducing-gpt-5/",
    name: "Introducing GPT-5",
    description: "OpenAI's flagship next-gen model launch",
    primaryCategorySlug: "model-providers",
  },
  {
    url: "https://openai.com/index/introducing-gpt-5-2/",
    name: "Introducing GPT-5.2",
    description: "Mid-cycle GPT-5 family update",
    primaryCategorySlug: "model-providers",
  },
  {
    url: "https://openai.com/index/sora-2/",
    name: "Sora 2",
    description: "Audio + physics video model with the Sora app (Sep 2025)",
    primaryCategorySlug: "model-providers",
    categorySlugs: ["model-providers", "multimodal"],
  },
  {
    url: "https://openai.com/index/introducing-codex/",
    name: "Introducing Codex",
    description: "codex-1 cloud coding agent (May 2025)",
    primaryCategorySlug: "coding-tools",
    categorySlugs: ["coding-tools", "model-providers"],
  },
  {
    url: "https://openai.com/index/codex-now-generally-available/",
    name: "Codex generally available",
    description: "Codex GA + SDK + Slack integration (Oct 2025)",
    primaryCategorySlug: "coding-tools",
  },
  {
    url: "https://blog.google/products/gemini/gemini-3/",
    name: "Gemini 3 announcement",
    description: "Google's flagship Gemini 3 + Antigravity (Nov 2025)",
    primaryCategorySlug: "model-providers",
  },
  {
    url: "https://antigravity.google/",
    name: "Google Antigravity",
    description: "Google's agentic IDE platform (Nov 2025)",
    primaryCategorySlug: "coding-tools",
  },
  {
    url: "https://x.ai/news",
    name: "xAI news (Grok 4)",
    description: "Grok 4 + Heavy multi-agent variant launch (Jul 2025)",
    primaryCategorySlug: "model-providers",
  },
  {
    url: "https://huggingface.co/deepseek-ai/DeepSeek-R1",
    name: "DeepSeek-R1 model card",
    description: "Open reasoning model rivaling o1 (Jan 2025)",
    primaryCategorySlug: "model-providers",
  },
  {
    url: "https://api-docs.deepseek.com/news/news251201",
    name: "DeepSeek-V3.2 release notes",
    description: "Latest V3.x line update from DeepSeek",
    primaryCategorySlug: "model-providers",
  },
  {
    url: "https://ai.meta.com/blog/llama-4-multimodal-intelligence/",
    name: "Llama 4 herd",
    description: "Scout + Maverick MoE models (Apr 2025)",
    primaryCategorySlug: "model-providers",
  },
  {
    url: "https://www.alibabacloud.com/blog/alibaba-introduces-qwen3-setting-new-benchmark-in-open-source-ai-with-hybrid-reasoning_602192",
    name: "Qwen3 launch",
    description: "Hybrid-reasoning Qwen3 family (Apr 2025)",
    primaryCategorySlug: "model-providers",
    categorySlugs: ["model-providers", "china"],
  },
  {
    url: "https://docs.z.ai/release-notes/new-released",
    name: "GLM-4.6 (Z.ai release notes)",
    description: "355B MIT-licensed coding model (Sep 2025)",
    primaryCategorySlug: "model-providers",
    categorySlugs: ["model-providers", "china"],
  },
  {
    url: "https://mistral.ai/news/mistral-medium-3",
    name: "Mistral Medium 3",
    description: "Cost-efficient frontier-class model (May 2025)",
    primaryCategorySlug: "model-providers",
  },
  {
    url: "https://mistral.ai/news/mistral-3",
    name: "Mistral Large 3",
    description: "Open-weight 675B-total MoE (Dec 2025)",
    primaryCategorySlug: "model-providers",
  },
  {
    url: "https://docs.cohere.com/v2/changelog/command-a",
    name: "Cohere Command A",
    description: "111B enterprise model w/ 256K context (Mar 2025)",
    primaryCategorySlug: "model-providers",
  },

  // ── Coding tools ────────────────────────────────────────
  {
    url: "https://cursor.com/changelog/1-0",
    name: "Cursor 1.0",
    description: "Bugbot, Background Agent GA, Memories (Jun 2025)",
    primaryCategorySlug: "coding-tools",
  },
  {
    url: "https://cursor.com/blog/2-0",
    name: "Cursor 2.0 + Composer",
    description: "In-house Composer model + multi-agent UI (Oct 2025)",
    primaryCategorySlug: "coding-tools",
  },
  {
    url: "https://cognition.ai/blog/introducing-devin-2-0",
    name: "Devin 2.0",
    description: "Cognition's revamped Devin at $20 starting tier (Apr 2025)",
    primaryCategorySlug: "coding-tools",
  },
  {
    url: "https://cognition.ai/blog/introducing-devin-2-2",
    name: "Devin 2.2",
    description: "Self-verifying, computer-use-testing Devin (Feb 2026)",
    primaryCategorySlug: "coding-tools",
  },
  {
    url: "https://blog.replit.com/introducing-agent-3-our-most-autonomous-agent-yet",
    name: "Replit Agent 3",
    description: "200-min autonomy + agent-spawning (Sep 2025)",
    primaryCategorySlug: "coding-tools",
  },
  {
    url: "https://vercel.com/blog/v0-app",
    name: "v0.app launch",
    description: "Full-stack agentic app builder (Aug 2025)",
    primaryCategorySlug: "coding-tools",
  },
  {
    url: "https://lovable.dev/blog/series-b",
    name: "Lovable Series B",
    description: "$330M at $6.6B; vibe-coding milestone (Dec 2025)",
    primaryCategorySlug: "coding-tools",
  },
  {
    url: "https://blog.stackblitz.com/posts/bolt-100k-oss-fund/",
    name: "Bolt 100K Open Source Fund",
    description: "StackBlitz Bolt ecosystem expansion",
    primaryCategorySlug: "coding-tools",
  },

  // ── Evals ───────────────────────────────────────────────
  {
    url: "https://metr.org/blog/2026-1-29-time-horizon-1-1/",
    name: "METR Time Horizons 1.1",
    description: "Autonomy time-horizon eval update (Jan 2026)",
    primaryCategorySlug: "evals",
  },
  {
    url: "https://www.tbench.ai/",
    name: "Terminal-Bench",
    description: "Real CLI-task agent benchmark; T-Bench 2.0 leaderboard",
    primaryCategorySlug: "evals",
  },
  {
    url: "https://gorilla.cs.berkeley.edu/blogs/13_bfcl_v3_multi_turn.html",
    name: "BFCL v3",
    description: "Berkeley multi-turn function-calling leaderboard",
    primaryCategorySlug: "evals",
  },
];

function kebab(s: string, maxWords = 8): string {
  return s
    .toLowerCase()
    .replace(/[—–]/g, "-")
    .replace(/[^\w\s-]/g, " ")
    .trim()
    .split(/\s+/)
    .slice(0, maxWords)
    .join("-")
    .replace(/-+/g, "-")
    .replace(/^-|-$/g, "");
}

async function main() {
  const url = process.env.NEON_DATABASE_URL ?? process.env.DATABASE_URL;
  if (!url) {
    console.error("missing NEON_DATABASE_URL / DATABASE_URL");
    process.exit(1);
  }
  const sql = neon(url);

  let inserted = 0;
  let skipped = 0;

  console.log(`Inserting ${NEW_LINKS.length} candidate links…`);

  for (const l of NEW_LINKS) {
    const slugBase = kebab(l.name) || "link";
    // Best-effort unique slug — try the base, then -2, -3 etc.
    let slug = slugBase;
    let attempt = 1;
    while (true) {
      const exists = await sql`
        SELECT 1 FROM links WHERE slug = ${slug} LIMIT 1
      `;
      if (exists.length === 0) break;
      attempt += 1;
      slug = `${slugBase}-${attempt}`;
      if (attempt > 50) break;
    }

    const cats = l.categorySlugs ?? [l.primaryCategorySlug];
    const r = await sql`
      INSERT INTO links (
        slug, name, url, description,
        primary_category_slug, category_slugs, status
      )
      VALUES (
        ${slug},
        ${l.name},
        ${l.url},
        ${l.description},
        ${l.primaryCategorySlug},
        ${cats},
        'active'
      )
      ON CONFLICT (url) DO NOTHING
      RETURNING id, name
    `;
    if (r.length > 0) {
      console.log(`  + ${l.name}`);
      inserted += 1;
    } else {
      console.log(`  -- skip (url exists) ${l.url}`);
      skipped += 1;
    }
  }

  console.log(`\nInserted: ${inserted}`);
  console.log(`Skipped (already in directory): ${skipped}`);

  const counts = await sql`
    SELECT status, COUNT(*)::int AS n
    FROM links
    GROUP BY status
    ORDER BY status
  `;
  console.log("\n=== Final status counts ===");
  for (const c of counts) {
    const row = c as { status: string; n: number };
    console.log(`  ${row.status.padEnd(10)} ${row.n}`);
  }
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
