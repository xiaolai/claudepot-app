/**
 * Diff our /links/ directory against major curated "awesome" lists.
 * Fetches each list's README, extracts every markdown link, filters
 * noise (badges, anchors, the list itself), normalizes URLs, and
 * reports what's in their list but not ours.
 *
 *   pnpm exec tsx --env-file=.env.local scripts/diff-awesome-lists.ts
 *
 * Read-only — does NOT mutate the DB. Output is a curator review
 * surface; pick the gaps worth filling and add via insert-new-links.ts.
 */

import { writeFileSync } from "node:fs";
import { neon } from "@neondatabase/serverless";

type ListSpec = { slug: string; repo: string; raw: string; topic: string };

const LISTS: ListSpec[] = [
  {
    slug: "awesome-claude-code",
    repo: "hesreallyhim/awesome-claude-code",
    raw: "https://raw.githubusercontent.com/hesreallyhim/awesome-claude-code/main/README.md",
    topic: "Claude Code: skills, hooks, commands, plugins, IDE",
  },
  {
    slug: "awesome-mcp-servers",
    repo: "punkpeye/awesome-mcp-servers",
    raw: "https://raw.githubusercontent.com/punkpeye/awesome-mcp-servers/main/README.md",
    topic: "MCP servers, registries, clients",
  },
  {
    slug: "Awesome-LLM",
    repo: "Hannibal046/Awesome-LLM",
    raw: "https://raw.githubusercontent.com/Hannibal046/Awesome-LLM/main/README.md",
    topic: "LLMs broadly: papers, models, tools, courses",
  },
  {
    slug: "awesome-llm-apps",
    repo: "Shubhamsaboo/awesome-llm-apps",
    raw: "https://raw.githubusercontent.com/Shubhamsaboo/awesome-llm-apps/main/README.md",
    topic: "LLM application examples and reference architectures",
  },
  {
    slug: "awesome-ai-agents",
    repo: "e2b-dev/awesome-ai-agents",
    raw: "https://raw.githubusercontent.com/e2b-dev/awesome-ai-agents/main/README.md",
    topic: "AI agent products and frameworks",
  },
  {
    slug: "awesome-claude-skills",
    repo: "travisvn/awesome-claude-skills",
    raw: "https://raw.githubusercontent.com/travisvn/awesome-claude-skills/main/README.md",
    topic: "Claude Skills (capability bundles)",
  },
];

const NOISE_HOSTS = new Set([
  "img.shields.io",
  "badges.gitter.im",
  "badge.fury.io",
  "raw.githubusercontent.com",
  "example.com",
  "www.example.com",
]);

const NOISE_PATH_PATTERNS = [
  /\/blob\/.*\.png$/i,
  /\/blob\/.*\.svg$/i,
  /\/blob\/.*\.gif$/i,
  /\/blob\/.*\.jpg$/i,
  /^\/contributors\/?$/i,
];

function normalize(u: string): string {
  try {
    const p = new URL(u.trim());
    let host = p.hostname.toLowerCase();
    if (host.startsWith("www.")) host = host.slice(4);
    let path = p.pathname.replace(/\/$/, "") || "";
    return `https://${host}${path}${p.search}`;
  } catch {
    return u.trim();
  }
}

const LINK_RE = /\[([^\]\n]+)\]\((https?:\/\/[^\s)]+)\)/g;

function extractLinks(md: string, ownRepo: string): { name: string; url: string }[] {
  const out: { name: string; url: string }[] = [];
  const seen = new Set<string>();
  for (const m of md.matchAll(LINK_RE)) {
    const name = m[1].trim();
    const url = m[2].trim().replace(/[),.]+$/, "");
    let p: URL;
    try {
      p = new URL(url);
    } catch {
      continue;
    }
    if (NOISE_HOSTS.has(p.hostname)) continue;
    if (NOISE_PATH_PATTERNS.some((re) => re.test(p.pathname))) continue;
    if (url.includes(ownRepo)) continue; // self-reference
    // Skip image-shaped names that suggest a badge/icon embed.
    if (/^!\[/.test(name) || name.length < 2) continue;
    const key = normalize(url);
    if (seen.has(key)) continue;
    seen.add(key);
    out.push({ name, url });
  }
  return out;
}

async function main() {
  const dbUrl = process.env.NEON_DATABASE_URL ?? process.env.DATABASE_URL;
  if (!dbUrl) {
    console.error("missing NEON_DATABASE_URL / DATABASE_URL");
    process.exit(1);
  }
  const sql = neon(dbUrl);

  // Pull every URL we have, regardless of status — archived URLs were
  // also previously curated, so a "missing" report shouldn't flag them
  // as new.
  const ourRows = (await sql`SELECT url FROM links`) as { url: string }[];
  const ours = new Set(ourRows.map((r) => normalize(r.url)));
  console.log(`Our directory: ${ours.size} unique URLs (active + archived)`);

  type Report = {
    list: ListSpec;
    total: number;
    overlap: number;
    missing: { name: string; url: string }[];
  };
  const reports: Report[] = [];

  for (const list of LISTS) {
    process.stdout.write(`Fetching ${list.slug}… `);
    let md: string;
    try {
      const res = await fetch(list.raw, {
        headers: { "User-Agent": "claudepot-diff/1.0" },
      });
      if (!res.ok) {
        console.log(`failed (${res.status})`);
        continue;
      }
      md = await res.text();
    } catch (e) {
      console.log(`failed (${(e as Error).message})`);
      continue;
    }
    const links = extractLinks(md, list.repo);
    const overlap = links.filter((l) => ours.has(normalize(l.url))).length;
    const missing = links.filter((l) => !ours.has(normalize(l.url)));
    console.log(
      `${links.length} links, ${overlap} overlap, ${missing.length} missing`,
    );
    reports.push({ list, total: links.length, overlap, missing });
  }

  // Summary table
  console.log("\n=== Per-list summary ===");
  console.log("list".padEnd(28) + "total".padEnd(8) + "overlap".padEnd(10) + "missing");
  for (const r of reports) {
    console.log(
      r.list.slug.padEnd(28) +
        String(r.total).padEnd(8) +
        String(r.overlap).padEnd(10) +
        String(r.missing.length),
    );
  }

  // Pooled missing URLs: how many appear in 2+ awesome lists? Those
  // are higher-confidence gaps.
  const missingFreq = new Map<
    string,
    { url: string; name: string; lists: string[] }
  >();
  for (const r of reports) {
    for (const m of r.missing) {
      const key = normalize(m.url);
      const cur =
        missingFreq.get(key) ?? { url: m.url, name: m.name, lists: [] };
      cur.lists.push(r.list.slug);
      missingFreq.set(key, cur);
    }
  }
  const multiList = [...missingFreq.values()]
    .filter((m) => m.lists.length > 1)
    .sort((a, b) => b.lists.length - a.lists.length);
  console.log(
    `\n=== Cross-list missing (in 2+ awesome lists, higher confidence) ===`,
  );
  console.log(`Count: ${multiList.length}`);
  for (const m of multiList.slice(0, 50)) {
    console.log(
      `  [${m.lists.length}x ${m.lists.slice(0, 3).join(",")}]`.padEnd(40) +
        ` ${m.name.slice(0, 40).padEnd(42)} ${m.url}`,
    );
  }

  // Per-list top missing (single-list, may be niche)
  console.log("\n=== Per-list top 20 missing (may include niche entries) ===");
  for (const r of reports) {
    console.log(`\n--- ${r.list.slug} (${r.missing.length} missing) ---`);
    for (const m of r.missing.slice(0, 20)) {
      console.log(`  ${m.name.slice(0, 50).padEnd(52)} ${m.url}`);
    }
  }

  writeFileSync(
    "/tmp/links-diff-awesome.json",
    JSON.stringify(
      { ranAt: new Date().toISOString(), reports, multiList },
      null,
      2,
    ),
  );
  console.log("\nFull report: /tmp/links-diff-awesome.json");
}

main().catch((e) => {
  console.error(e);
  process.exit(1);
});
