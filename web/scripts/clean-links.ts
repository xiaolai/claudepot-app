/**
 * Apply curator decisions from /tmp/links-verify.json:
 *
 *   ARCHIVE_URLS  — confirmed-dead URLs flipped to status='archived'
 *                   (reversible; no row deletion).
 *   URL_UPDATES   — current canonical URL for known domain/path moves.
 *                   If the new URL already exists in the DB (collision),
 *                   the old row is archived instead of double-inserting.
 *
 * Idempotent: every UPDATE matches by URL; re-runs are no-ops.
 *
 *   pnpm exec tsx --env-file=.env.local scripts/clean-links.ts
 *
 * Hand-curated subset of the 91 substantive redirects flagged by the
 * verifier. Trailing-slash and discord.gg → discord.com/invite
 * redirects are intentionally skipped (the old URL still works; no
 * functional gain from rewriting).
 */

import { neon } from "@neondatabase/serverless";

const ARCHIVE_URLS = [
  // 404 GitHub repos with no obvious successor.
  "https://github.com/run-llama/awesome-llamaindex",
  "https://github.com/Aman-4-Real/Awesome-LLMs-Papers",
  // ChatGPT search is built into chatgpt.com — no separate /search route.
  "https://chatgpt.com/search",
];

// [oldUrl, newUrl] — applied in order; collisions auto-archive the old row.
const URL_UPDATES: [string, string][] = [
  // ── Anthropic → claude.com domain consolidation ────────────
  ["https://support.anthropic.com", "https://support.claude.com"],
  ["https://docs.anthropic.com", "https://platform.claude.com/docs/en/home"],
  [
    "https://docs.anthropic.com/en/docs/build-with-claude/prompt-engineering/overview",
    "https://platform.claude.com/docs/en/build-with-claude/prompt-engineering/overview",
  ],
  [
    "https://docs.anthropic.com/en/docs/build-with-claude/token-counting",
    "https://platform.claude.com/docs/en/build-with-claude/token-counting",
  ],
  [
    "https://docs.anthropic.com/en/resources/prompt-library/library",
    "https://platform.claude.com/docs/en/build-with-claude/prompt-engineering/claude-prompting-best-practices",
  ],
  [
    "https://docs.claude.com/en/api/agent-sdk/overview",
    "https://code.claude.com/docs/en/agent-sdk/overview",
  ],
  [
    "https://www.anthropic.com/rsp-updates",
    "https://www.anthropic.com/responsible-scaling-policy",
  ],
  [
    "https://www.anthropic.com/legal/usage-policy",
    "https://www.anthropic.com/legal/aup",
  ],
  [
    "https://www.anthropic.com/research/building-effective-agents",
    "https://www.anthropic.com/engineering/building-effective-agents",
  ],
  [
    "https://www.anthropic.com/news/skills",
    "https://claude.com/blog/skills",
  ],
  ["https://claude.ai/download", "https://claude.com/download"],
  // anthropic-cookbook repo collides with claude-cookbooks already in DB.
  // Old row is archived by collision-handler below.
  [
    "https://github.com/anthropics/anthropic-cookbook",
    "https://github.com/anthropics/claude-cookbooks",
  ],

  // ── LMArena → Arena rebrand ────────────────────────────────
  ["https://lmarena.ai", "https://arena.ai/"],
  ["https://lmarena.ai/?mode=direct", "https://arena.ai/text/direct"],
  ["https://lmarena.ai/video", "https://arena.ai/video"],
  [
    "https://lmarena.ai/leaderboard/text-to-image",
    "https://arena.ai/leaderboard/text-to-image",
  ],
  [
    "https://lmarena.ai/leaderboard/text-to-video",
    "https://arena.ai/leaderboard/text-to-video",
  ],
  [
    "https://lmarena.ai/leaderboard/image-edit",
    "https://arena.ai/leaderboard/image-edit",
  ],

  // ── GitHub repo org/name renames ───────────────────────────
  [
    "https://github.com/comfyanonymous/ComfyUI",
    "https://github.com/Comfy-Org/ComfyUI",
  ],
  [
    "https://github.com/ltdrdata/ComfyUI-Manager",
    "https://github.com/Comfy-Org/ComfyUI-Manager",
  ],
  [
    "https://github.com/hiyouga/LLaMA-Factory",
    "https://github.com/hiyouga/LlamaFactory",
  ],
  [
    "https://github.com/mendableai/firecrawl-mcp-server",
    "https://github.com/firecrawl/firecrawl-mcp-server",
  ],
  [
    "https://github.com/apify/actors-mcp-server",
    "https://github.com/apify/apify-mcp-server",
  ],
  [
    "https://github.com/stanford-crfm/levanter",
    "https://github.com/marin-community/levanter",
  ],

  // ── Service rebrands / acquisitions ────────────────────────
  // AI Snake Oil's Substack rebranded to Normal Tech.
  ["https://www.aisnakeoil.com/", "https://www.normaltech.ai/"],
  // Msty rebranded msty.app → msty.ai.
  ["https://msty.app/", "https://msty.ai/"],
  // Lepton AI was acquired by NVIDIA (now DGX Cloud Lepton).
  [
    "https://www.lepton.ai",
    "https://www.nvidia.com/en-us/data-center/dgx-cloud-lepton/",
  ],
  // OpenAI Cookbook moved to developers.openai.com.
  [
    "https://cookbook.openai.com/",
    "https://developers.openai.com/cookbook",
  ],
  [
    "https://cookbook.openai.com/examples/gpt-5/gpt-5_prompting_guide",
    "https://developers.openai.com/cookbook/examples/gpt-5/gpt-5_prompting_guide",
  ],
  // ai.engineer apex → www subdomain.
  ["https://ai.engineer", "https://www.ai.engineer"],
  // MCP Quickstart moved under /docs/develop.
  [
    "https://modelcontextprotocol.io/quickstart/server",
    "https://modelcontextprotocol.io/docs/develop/build-server",
  ],
  // LangChain.js docs moved to the unified langchain.com site.
  [
    "https://js.langchain.com",
    "https://docs.langchain.com/oss/javascript/langchain/overview",
  ],
  // LlamaIndex.TS moved to developers.llamaindex.ai.
  [
    "https://ts.llamaindex.ai",
    "https://developers.llamaindex.ai/typescript/framework/",
  ],
];

async function main() {
  const url = process.env.NEON_DATABASE_URL ?? process.env.DATABASE_URL;
  if (!url) {
    console.error("missing NEON_DATABASE_URL / DATABASE_URL");
    process.exit(1);
  }
  const sql = neon(url);

  let archivedCount = 0;
  let updatedCount = 0;
  let collisionArchivedCount = 0;
  let skipped = 0;

  console.log("=== Archiving dead URLs ===");
  for (const u of ARCHIVE_URLS) {
    const r = await sql`
      UPDATE links
      SET status = 'archived', updated_at = NOW()
      WHERE url = ${u} AND status <> 'archived'
      RETURNING name
    `;
    if (r.length > 0) {
      console.log(`  archived  ${(r[0] as { name: string }).name}`);
      archivedCount += 1;
    } else {
      skipped += 1;
    }
  }

  console.log("\n=== Updating moved URLs ===");
  for (const [oldUrl, newUrl] of URL_UPDATES) {
    const collide = await sql`
      SELECT 1 FROM links WHERE url = ${newUrl} LIMIT 1
    `;
    if (collide.length > 0) {
      // Target already in directory — archive the old row instead of
      // creating a phantom dup.
      const arch = await sql`
        UPDATE links
        SET status = 'archived', updated_at = NOW()
        WHERE url = ${oldUrl} AND status <> 'archived'
        RETURNING name
      `;
      if (arch.length > 0) {
        console.log(`  collision-archived  ${(arch[0] as { name: string }).name}`);
        console.log(`            old: ${oldUrl}`);
        console.log(`            new: ${newUrl}  (already in directory)`);
        collisionArchivedCount += 1;
      } else {
        skipped += 1;
      }
      continue;
    }
    const r = await sql`
      UPDATE links
      SET url = ${newUrl}, updated_at = NOW()
      WHERE url = ${oldUrl}
      RETURNING name
    `;
    if (r.length > 0) {
      console.log(`  moved  ${(r[0] as { name: string }).name}`);
      console.log(`         ${oldUrl}`);
      console.log(`      →  ${newUrl}`);
      updatedCount += 1;
    } else {
      skipped += 1;
    }
  }

  console.log("\n=== Summary ===");
  console.log(`  archived (dead):       ${archivedCount}`);
  console.log(`  archived (collision):  ${collisionArchivedCount}`);
  console.log(`  url updated:           ${updatedCount}`);
  console.log(`  skipped (no-op):       ${skipped}`);

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
