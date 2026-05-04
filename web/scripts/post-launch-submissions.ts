/**
 * Curated launch-day backfill from design/fixtures/submissions-foxed.json.
 *
 *   pnpm exec tsx --env-file=.env.local scripts/post-launch-submissions.ts
 *
 * Picks 12 high-signal items by URL match — a healthy mix of Anthropic
 * official news, Claude Code resources, builder analysis, and one
 * benchmark. Skips drama/complaint posts; the audience is builders,
 * not gossip readers.
 *
 * Each item is inserted as state='approved' so it appears on /, /new,
 * /top immediately. createdAt and publishedAt are stamped at NOW
 * (staggered 7 min apart in `URLS_TO_PICK` order) — the fixture's
 * submitted_at would back-date items by days/weeks and Hot's age
 * decay would bury them under whatever fresher rows already exist.
 * Tags are taken from the fixture's `tags` array but filtered to the
 * 11-tag taxonomy in the DB (the fixture is noisy — e.g. "5x5 Pixel
 * font" is mistagged `mcp`). Score is upvotes − downvotes.
 *
 * Idempotent: skips any URL that already exists in the submissions
 * table, so it's safe to re-run while iterating on the curation list.
 */

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { eq, inArray } from "drizzle-orm";

import { db } from "@/db/client";
import { submissions, submissionTags, tags, users } from "@/db/schema";

type FoxedItem = {
  id: string;
  user: string;
  type: string;
  tags: string[];
  title: string;
  url: string;
  upvotes: number;
  downvotes: number;
  submitted_at: string;
  reading_time_min: number | null;
};

const URLS_TO_PICK = [
  "https://www.anthropic.com/glasswing",
  "https://ccunpacked.dev/",
  "https://code.claude.com/docs/en/routines",
  "https://simonwillison.net/2026/Apr/18/opus-system-prompt/",
  "https://simonwillison.net/2026/Apr/16/qwen-beats-opus/",
  "https://mtlynch.io/claude-code-found-linux-vulnerability/",
  "https://david.coffee/i-still-prefer-mcp-over-skills/",
  "https://www.anthropic.com/news/google-broadcom-partnership-compute",
  "https://www-cdn.anthropic.com/53566bf5440a10affd749724787c8913a2ae0841.pdf",
  "https://www.claudecodecamp.com/p/i-measured-claude-4-7-s-new-tokenizer-here-s-what-it-costs-you",
  "https://github.com/dirac-run/dirac",
  "https://braw.dev/blog/2026-04-06-reallocating-100-month-claude-spend/",
];

// foxed `type` → submissions.type enum. The fixture uses 'news' as a
// catch-all, but claudepot.com's enum exposes nine specific types — keep
// 'news', 'article', 'discussion', 'tool', 'podcast' as-is.
const TYPE_MAP: Record<string, string> = {
  news: "news",
  article: "article",
  discussion: "discussion",
  tool: "tool",
  podcast: "podcast",
};

async function main() {
  const path = resolve(process.cwd(), "design/fixtures/submissions-foxed.json");
  const all: FoxedItem[] = JSON.parse(readFileSync(path, "utf8"));

  // 1. Resolve picks.
  const picks: FoxedItem[] = [];
  const missing: string[] = [];
  for (const url of URLS_TO_PICK) {
    const hit = all.find((it) => it.url === url);
    if (!hit) {
      missing.push(url);
      continue;
    }
    picks.push(hit);
  }
  if (missing.length) {
    console.error(`could not resolve ${missing.length} URL(s):`);
    for (const u of missing) console.error(`  ${u}`);
    process.exit(1);
  }

  // 2. Build the user → id map.
  const usernames = Array.from(new Set(picks.map((p) => p.user)));
  const userRows = await db
    .select({ id: users.id, username: users.username })
    .from(users)
    .where(inArray(users.username, usernames));
  const usernameToId = new Map(userRows.map((r) => [r.username, r.id]));
  const missingUsers = usernames.filter((u) => !usernameToId.has(u));
  if (missingUsers.length) {
    console.error(`missing users: ${missingUsers.join(", ")}`);
    process.exit(1);
  }

  // 3. Load valid tag slugs for the filter.
  const tagRows = await db.select({ slug: tags.slug }).from(tags);
  const validTags = new Set(tagRows.map((r) => r.slug));

  // 4. Skip URLs that already landed.
  const existingUrls = new Set<string>(
    (
      await db
        .select({ url: submissions.url })
        .from(submissions)
        .where(inArray(submissions.url, URLS_TO_PICK))
    ).map((r) => r.url ?? ""),
  );

  let inserted = 0;
  let skipped = 0;
  for (const it of picks) {
    if (it.url && existingUrls.has(it.url)) {
      skipped += 1;
      console.log(`  skip (exists)  ${it.title.slice(0, 70)}`);
      continue;
    }

    const authorId = usernameToId.get(it.user)!;
    const score = (it.upvotes ?? 0) - (it.downvotes ?? 0);
    const type = TYPE_MAP[it.type] ?? "news";
    // Stagger 7 min apart in the URL-list order so the highest-priority
    // pick is most recent (Hot rank stays in priority order) and /new
    // doesn't hand out an arbitrary order to identical timestamps.
    const ts = new Date(Date.now() - inserted * 7 * 60 * 1000);

    const [row] = await db
      .insert(submissions)
      .values({
        authorId,
        type: type as never,
        title: it.title,
        url: it.url,
        text: null,
        state: "approved",
        score,
        readingTimeMin: it.reading_time_min ?? null,
        createdAt: ts,
        publishedAt: ts,
      })
      .returning({ id: submissions.id });

    const itTags = (it.tags ?? []).filter((t) => validTags.has(t));
    if (itTags.length > 0) {
      await db
        .insert(submissionTags)
        .values(itTags.map((slug) => ({ submissionId: row.id, tagSlug: slug })))
        .onConflictDoNothing();
    }

    inserted += 1;
    console.log(
      `  insert ▲${score.toString().padStart(4)}  ${type.padEnd(10)} @${it.user.padEnd(8)} ${it.title.slice(0, 70)}`,
    );
  }

  console.log(`\n— done. inserted ${inserted}, skipped ${skipped}.`);
}

await main();
