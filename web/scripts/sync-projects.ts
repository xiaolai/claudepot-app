/**
 * Pull xiaolai's GitHub repos via `gh repo list`, filter to AI-tooling
 * non-forks, and write design/fixtures/projects.json.
 *
 *   pnpm tsx scripts/sync-projects.ts          # writes the JSON
 *   pnpm tsx scripts/sync-projects.ts --seed   # also seeds the DB
 *
 * Filter logic:
 *   - non-fork (`gh --source` flag in the upstream call)
 *   - non-archived
 *   - description matches AI/Claude/agent/MCP keywords
 *   - explicit deny-list (books, personal sites, finance scripts)
 *
 * The output JSON is the source of truth for /projects. Hand-edit the
 * file to add a project that didn't pass the filter (or remove one
 * that did but shouldn't).
 */

import { execSync } from "node:child_process";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";

const FIXTURE_PATH = resolve(process.cwd(), "design/fixtures/projects.json");
// Optional editorial mapping: { "<slug>": ["tag-slug", ...] }. Hand-
// authored. The seed step inserts these into project_tags. Missing
// slugs are simply not bound to any tags — the detail page renders an
// empty-state CTA, which is the prompt to populate this file.
const PROJECT_TAGS_PATH = resolve(
  process.cwd(),
  "design/fixtures/project-tags.json",
);

interface GhRepo {
  name: string;
  description: string | null;
  primaryLanguage: { name: string } | null;
  stargazerCount: number;
  url: string;
  updatedAt: string;
  isPrivate?: boolean;
}

interface ProjectFixture {
  slug: string;
  name: string;
  tagline: string;
  repo_url: string;
  site_url: string | null;
  primary_language: string | null;
  stars: number;
  updated_at: string;
}

const KEYWORDS =
  /claude|llm|\bai\b|agent|prompt|mcp|nlp|gpt|anthropic|\bskill\b|hook|tokeniz|codex|terminal/i;

const DENY = new Set([
  // Books, personal sites, finance, dotfiles
  "lixiaolai.com",
  "lixiaolai.com-old",
  "regular-investing-in-box",
  "homebrew-tap",
  "homebrew-chrome-private-bridge",
  "lixiaolai-com-issues",
  "lixiaolai-import-test",
  "myhexo",
  "iterm-config",
  "rime-settings",
  "my-dotfiles",
  "surge-conf",
  "english-course",
  // Books that mention AI/ChatGPT in descriptions but aren't tools
  "most-common-american-idioms",
  "little-book-of-ai",
  // Old / unrelated
  "kubesphere",
  "cli",
  "extract-histical-data-from-yahoo-finance",
  "monte-carlo-projection",
  "sp500-minimal-l",
  "sp500-convergence",
  "Window-Management-alfredworkflow",
  "prezicnfonts",
  "Toddler-Picture-Story-Generator", // fun but old, optional re-add
  // Test fixtures, notes, dotfile dumps
  "nlpm-test-fixture",
  "claude-cli-notes",
  "how-do-we-use-claude-code",
  "dot-claude",
  "open-terminal-at-vscode-without-folder-specified",
  "toggle-chat-list",
]);

const ALLOW = new Set([
  // Force-include — these are AI-tooling projects whose descriptions
  // happen not to match the keyword filter (tool-agnostic phrasing,
  // CJK-only, etc.). Add a slug here when you spot one missing.
  "vmark",
  "sha.com",
  "claudepot-app",
  "mecha.im",
  "tepub",
  "ui-responsive",
]);

/**
 * Fetch a repo's README.md as plain markdown. The GitHub API endpoint
 * returns base64-encoded `content`; we decode it. 404 (no README) and
 * any other error fall back to null so the seed loop continues.
 */
function fetchReadme(repoName: string): string | null {
  try {
    const json = execSync(
      `gh api repos/xiaolai/${repoName}/readme --header "Accept: application/vnd.github+json"`,
      { encoding: "utf-8", stdio: ["ignore", "pipe", "ignore"] },
    );
    const data = JSON.parse(json) as { content?: string; encoding?: string };
    if (data.encoding === "base64" && data.content) {
      return Buffer.from(data.content, "base64").toString("utf-8");
    }
    return null;
  } catch {
    return null;
  }
}

function fetchRepos(): GhRepo[] {
  console.log("→ Fetching public repos via gh CLI…");
  // --visibility public filters out private/internal repos at fetch time.
  // --source drops forks. --no-archived drops archived.
  const json = execSync(
    "gh repo list xiaolai --limit 200 --no-archived --source --visibility public --json name,description,primaryLanguage,stargazerCount,url,updatedAt,isPrivate",
    { encoding: "utf-8" },
  );
  const repos = JSON.parse(json) as Array<GhRepo & { isPrivate?: boolean }>;
  // Belt-and-suspenders: even with --visibility public, drop any flagged private.
  return repos.filter((r) => !r.isPrivate);
}

function filter(repos: GhRepo[]): GhRepo[] {
  return repos.filter((r) => {
    if (DENY.has(r.name)) return false;
    if (ALLOW.has(r.name)) return true;
    const blob = `${r.name} ${r.description ?? ""}`;
    return KEYWORDS.test(blob);
  });
}

function toFixture(r: GhRepo): ProjectFixture {
  return {
    slug: r.name.toLowerCase(),
    name: r.name,
    tagline: r.description ?? "",
    repo_url: r.url,
    site_url: null,
    primary_language: r.primaryLanguage?.name ?? null,
    stars: r.stargazerCount,
    updated_at: r.updatedAt,
  };
}

const repos = fetchRepos();
console.log(`  fetched ${repos.length} repos`);

const filtered = filter(repos);
console.log(`  filtered → ${filtered.length} projects after keyword + allow/deny`);

const projectsList = filtered
  .map(toFixture)
  .sort((a, b) => b.stars - a.stars);

writeFileSync(FIXTURE_PATH, JSON.stringify(projectsList, null, 2) + "\n");
console.log(`✓ wrote ${projectsList.length} entries to ${FIXTURE_PATH}`);

const SEED = process.argv.includes("--seed");
if (!SEED) {
  console.log("  → run with --seed to also INSERT into the DB");
  process.exit(0);
}

/* ── Seed (idempotent; lazy DB import so sync alone doesn't need creds) */

console.log(`→ Seeding ${projectsList.length} projects…`);

const { db } = await import("@/db/client");
const { sql } = await import("drizzle-orm");

const ownerResult = await db.execute(
  sql`SELECT id FROM users WHERE username = 'xiaolai' LIMIT 1`,
);
// eslint-disable-next-line @typescript-eslint/no-explicit-any
const ownerRows = (ownerResult as any).rows ?? ownerResult;
const ownerRow = ownerRows[0] as { id: string } | undefined;
if (!ownerRow) {
  console.error(
    "✗ no @xiaolai user — seed users first (pnpm tsx --env-file=.env.local scripts/seed-agents.ts or main seed.ts)",
  );
  process.exit(1);
}
const ownerId = ownerRow.id;

let inserted = 0;
let updated = 0;
let readmeHits = 0;
let readmeMisses = 0;
for (const p of projectsList) {
  const readme = fetchReadme(p.name);
  if (readme) readmeHits++;
  else readmeMisses++;
  const result = await db.execute(sql`
    INSERT INTO projects (slug, name, blurb, owner_id, repo_url, site_url, primary_language, stars, updated_at, readme_md)
    VALUES (
      ${p.slug}, ${p.name}, ${p.tagline}, ${ownerId},
      ${p.repo_url}, ${p.site_url}, ${p.primary_language}, ${p.stars}, ${p.updated_at},
      ${readme}
    )
    ON CONFLICT (slug) DO UPDATE SET
      name = EXCLUDED.name,
      blurb = EXCLUDED.blurb,
      repo_url = EXCLUDED.repo_url,
      primary_language = EXCLUDED.primary_language,
      stars = EXCLUDED.stars,
      updated_at = EXCLUDED.updated_at,
      readme_md = EXCLUDED.readme_md
    RETURNING (xmax = 0) AS inserted
  `);
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  const row = ((result as any).rows ?? result)[0] as { inserted: boolean };
  if (row?.inserted) inserted++;
  else updated++;
}

console.log(
  `✓ ${inserted} inserted, ${updated} updated · README ${readmeHits} hit, ${readmeMisses} miss`,
);

// Optional project_tags seed. The mapping file is editorial — present
// only when xiaolai has curated tags for one or more projects. We
// REPLACE the bound set per project listed, leaving unlisted projects
// untouched (lets ad-hoc tag edits in the DB survive a re-seed).
if (existsSync(PROJECT_TAGS_PATH)) {
  const raw = JSON.parse(readFileSync(PROJECT_TAGS_PATH, "utf-8")) as Record<
    string,
    string[]
  >;
  let bound = 0;
  let missing = 0;
  for (const [slug, tagSlugs] of Object.entries(raw)) {
    const idResult = await db.execute(
      sql`SELECT id FROM projects WHERE slug = ${slug} LIMIT 1`,
    );
    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const idRows = ((idResult as any).rows ?? idResult) as Array<{ id: string }>;
    const projectId = idRows[0]?.id;
    if (!projectId) {
      missing++;
      continue;
    }
    await db.execute(
      sql`DELETE FROM project_tags WHERE project_id = ${projectId}`,
    );
    for (const tagSlug of tagSlugs) {
      await db.execute(
        sql`INSERT INTO project_tags (project_id, tag_slug)
            VALUES (${projectId}, ${tagSlug})
            ON CONFLICT DO NOTHING`,
      );
      bound++;
    }
  }
  console.log(
    `✓ project_tags: bound ${bound} pairs across ${Object.keys(raw).length} projects (${missing} unknown slugs skipped)`,
  );
} else {
  console.log(
    `  no ${PROJECT_TAGS_PATH.replace(process.cwd() + "/", "")} — skipping project_tags seed`,
  );
}

// Sweep: remove projects that are no longer in the public-AI-tooling set.
// Repos that flipped to private, got renamed, or that we removed from the
// allow-list should not linger in the DB. project_submissions FKs cascade.
const slugs = projectsList.map((p) => p.slug);
const sweepResult = await db.execute(sql`
  DELETE FROM projects
  WHERE slug NOT IN (${sql.join(slugs.map((s) => sql`${s}`), sql`, `)})
  RETURNING slug
`);
// eslint-disable-next-line @typescript-eslint/no-explicit-any
const swept = ((sweepResult as any).rows ?? sweepResult) as Array<{ slug: string }>;
if (swept.length) {
  console.log(`✓ swept ${swept.length} stale entries: ${swept.map((s) => s.slug).join(", ")}`);
} else {
  console.log("✓ no stale entries to sweep");
}
