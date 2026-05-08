/**
 * Add the 10 high-signal entries surfaced by the
 * `diff-awesome-lists.ts` cross-check that the original seed missed.
 *
 *   pnpm exec tsx --env-file=.env.local scripts/insert-awesome-diff-picks.ts
 *
 * Idempotent via ON CONFLICT (url) DO NOTHING.
 */

import { neon } from "@neondatabase/serverless";

type NewLink = {
  url: string;
  name: string;
  description: string;
  primaryCategorySlug: string;
  categorySlugs?: string[];
};

const PICKS: NewLink[] = [
  {
    url: "https://lilianweng.github.io/posts/2023-06-23-agent/",
    name: "LLM Powered Autonomous Agents",
    description: "Lilian Weng's 2023 reference essay on agent design",
    primaryCategorySlug: "learning",
  },
  {
    url: "https://www.camel-ai.org/",
    name: "CAMEL",
    description: "Multi-agent communication framework + research community",
    primaryCategorySlug: "coding-tools",
  },
  {
    url: "https://github.com/langroid/langroid",
    name: "Langroid",
    description: "Python multi-agent framework with type-safe message passing",
    primaryCategorySlug: "coding-tools",
  },
  {
    url: "https://platform.claude.com/docs/en/api/beta/skills",
    name: "Skills API (docs)",
    description: "Beta API for managing Claude Skills programmatically",
    primaryCategorySlug: "skills",
    categorySlugs: ["skills", "anthropic"],
  },
  {
    url: "https://github.com/obra/superpowers-skills",
    name: "obra/superpowers-skills",
    description: "Jesse Vincent's expanded skill collection (companion to superpowers)",
    primaryCategorySlug: "skills",
  },
  {
    url: "https://github.com/huggingface/open-r1",
    name: "open-r1 (Hugging Face)",
    description: "HF's open replication of DeepSeek-R1's reasoning training",
    primaryCategorySlug: "model-providers",
  },
  {
    url: "https://github.com/Jiayi-Pan/TinyZero",
    name: "TinyZero",
    description: "Minimal DeepSeek-R1-style reasoning training reproduction",
    primaryCategorySlug: "model-providers",
  },
  {
    url: "https://github.com/MoonshotAI/Kimi-K2",
    name: "Kimi-K2 (GitHub)",
    description: "Moonshot's open agentic Kimi K2 series",
    primaryCategorySlug: "model-providers",
    categorySlugs: ["model-providers", "china"],
  },
  {
    url: "https://github.com/ericbuess/claude-code-docs",
    name: "claude-code-docs (offline mirror)",
    description: "Popular community mirror of Claude Code docs for offline use",
    primaryCategorySlug: "anthropic",
  },
  {
    url: "https://github.com/K-Dense-AI/claude-scientific-skills",
    name: "Claude Scientific Skills",
    description: "Skill bundle for chemistry/biology/physics research workflows",
    primaryCategorySlug: "skills",
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

  for (const l of PICKS) {
    const slugBase = kebab(l.name) || "link";
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
        ${slug}, ${l.name}, ${l.url}, ${l.description},
        ${l.primaryCategorySlug}, ${cats}, 'active'
      )
      ON CONFLICT (url) DO NOTHING
      RETURNING name
    `;
    if (r.length > 0) {
      console.log(`  + ${l.name}`);
      inserted += 1;
    } else {
      console.log(`  -- skip (already in) ${l.url}`);
      skipped += 1;
    }
  }

  console.log(`\nInserted: ${inserted}, skipped: ${skipped}`);
  const counts = await sql`
    SELECT status, COUNT(*)::int AS n FROM links GROUP BY status ORDER BY status
  `;
  console.log("\nStatus counts:");
  for (const c of counts) {
    const row = c as { status: string; n: number };
    console.log(`  ${row.status.padEnd(10)} ${row.n}`);
  }
}

main().catch((e) => { console.error(e); process.exit(1); });
