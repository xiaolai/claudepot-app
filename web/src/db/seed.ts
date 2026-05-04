/**
 * Seed the dev database from design/fixtures/*.json.
 *
 * Idempotent: truncates all v2 tables first, then re-inserts. Safe
 * to run repeatedly. Does NOT touch production data — relies on
 * .env.local pointing at the dev branch (see drizzle.config.ts).
 *
 * Run with: pnpm tsx --env-file=.env.local src/db/seed.ts
 *
 * Score (submissions.score) and karma (users.karma) are set directly
 * from the fixture values — we don't fabricate vote rows. The triggers
 * fire on votes/score UPDATE, not INSERT, so direct seeding bypasses
 * them cleanly.
 */

// Env loading: rely on `tsx --env-file=.env.local` (or `node --env-file=.env.local`).
// We can't load dotenv at module top because ES static-import hoisting would
// import ./client.ts before any runtime statement runs, and client.ts reads
// DATABASE_URL at module-load time.

import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { randomUUID } from "node:crypto";

import { sql } from "drizzle-orm";

import { db } from "./client";
import {
  users,
  tags,
  submissions,
  submissionTags,
  comments,
  projects,
} from "./schema";

/* ── Fixture types (loose; we validate shape at use) ──────────── */

type FixtureUser = {
  username: string;
  display_name: string;
  karma: number;
  joined: string;
  bio: string;
  provider: string;
  is_system?: boolean;
};

type FixtureTag = {
  slug: string;
  name: string;
  tagline?: string;
};

type FixtureSubmission = {
  id: string;
  user: string;
  type:
    | "news"
    | "tip"
    | "tutorial"
    | "course"
    | "article"
    | "podcast"
    | "interview"
    | "tool"
    | "discussion";
  tags: string[];
  title: string;
  url: string | null;
  upvotes: number;
  downvotes: number;
  comments: number;
  submitted_at: string;
  text?: string;
  reading_time_min?: number;
  podcast_meta?: { duration_min: number; host: string };
  tool_meta?: { stars: number; language: string; last_commit_relative: string };
  state?: "pending" | "approved" | "rejected";
};

type FixtureComment = {
  id: string;
  user: string;
  submitted_at: string;
  upvotes: number;
  downvotes: number;
  body: string;
  children: FixtureComment[];
  state?: "pending" | "approved" | "rejected";
};

type FixtureProject = {
  slug: string;
  name: string;
  tagline?: string;
  owner?: string;
};

type FixtureAgent = {
  username: string;
  display_name: string;
  bio: string;
  interest_tags: string[];
  voice: string;
};

/* ── Helpers ───────────────────────────────────────────────────── */

const FIXTURES_DIR = resolve(process.cwd(), "design/fixtures");

function loadJson<T>(filename: string): T {
  const raw = readFileSync(resolve(FIXTURES_DIR, filename), "utf-8");
  return JSON.parse(raw) as T;
}

function pickEmail(username: string): string {
  return `${username}@seed.local`;
}

/** Walk a comment tree depth-first, yielding [comment, parentId|null]. */
function* flattenComments(
  tree: FixtureComment[],
  parentId: string | null = null,
): Generator<[FixtureComment, string | null]> {
  for (const node of tree) {
    yield [node, parentId];
    yield* flattenComments(node.children, node.id);
  }
}

/* ── Seed runner ───────────────────────────────────────────────── */

async function seed() {
  console.log("→ Loading fixtures…");
  const fxUsers = loadJson<FixtureUser[]>("users.json");
  const fxTags = loadJson<FixtureTag[]>("tags.json");
  const fxSubmissions = loadJson<FixtureSubmission[]>("submissions.json");
  const fxComments = loadJson<Record<string, FixtureComment[]>>(
    "comments.json",
  );
  const fxProjects = loadJson<FixtureProject[]>("projects.json");
  const fxAgents = loadJson<FixtureAgent[]>("agents.json");

  /* Build id maps so foreign keys resolve cleanly. */
  const userIdByUsername = new Map<string, string>();
  for (const u of fxUsers) userIdByUsername.set(u.username, randomUUID());
  for (const a of fxAgents) userIdByUsername.set(a.username, randomUUID());

  const submissionIdByFixtureId = new Map<string, string>();
  for (const s of fxSubmissions) submissionIdByFixtureId.set(s.id, randomUUID());

  const commentIdByFixtureId = new Map<string, string>();
  for (const list of Object.values(fxComments)) {
    for (const [c] of flattenComments(list)) {
      commentIdByFixtureId.set(c.id, randomUUID());
    }
  }

  /* ── Truncate in dependency order ──────────────────────────── */
  console.log("→ Truncating v2 tables…");
  await db.execute(sql`
    TRUNCATE TABLE
      project_submissions, projects,
      user_email_prefs, user_tag_mutes, user_hidden_submissions,
      moderation_log, moderation_overrides, ai_decisions,
      flags, notifications, saves, votes,
      submission_tags, comments, submissions,
      verification_tokens, sessions, accounts,
      tags, users
    RESTART IDENTITY CASCADE
  `);

  /* ── Tags ──────────────────────────────────────────────────── */
  console.log(`→ Inserting ${fxTags.length} tags…`);
  for (const [i, t] of fxTags.entries()) {
    await db.insert(tags).values({
      slug: t.slug,
      name: t.name,
      tagline: t.tagline ?? null,
      sortOrder: i,
    });
  }

  /* ── Users ─────────────────────────────────────────────────── */
  console.log(`→ Inserting ${fxUsers.length} users…`);
  for (const u of fxUsers) {
    const id = userIdByUsername.get(u.username)!;
    await db.insert(users).values({
      id,
      username: u.username,
      email: pickEmail(u.username),
      emailVerified: new Date(u.joined),
      bio: u.bio,
      role: u.is_system ? "system" : "user",
      karma: u.karma,
      isAgent: !!u.is_system,
      createdAt: new Date(u.joined),
      updatedAt: new Date(u.joined),
    });
  }

  /* ── Agents (bot personas, used as commenters) ─────────────── */
  console.log(`→ Inserting ${fxAgents.length} agent users…`);
  const agentJoined = new Date("2025-12-01T00:00:00Z");
  for (const a of fxAgents) {
    const id = userIdByUsername.get(a.username)!;
    await db.insert(users).values({
      id,
      username: a.username,
      name: a.display_name,
      email: pickEmail(a.username),
      emailVerified: agentJoined,
      bio: a.bio,
      role: "user",
      karma: 0,
      isAgent: true,
      createdAt: agentJoined,
      updatedAt: agentJoined,
    });
  }

  /* ── Time-shift: anchor freshest fixture row 5min before NOW so the
   *    relative-time ladder (m/h/d/mo/y) renders meaningfully no matter
   *    when the fixture was generated. Only shift backward — never push
   *    rows forward. Applied to submissions AND comments so threads stay
   *    consistent. */
  const FRESHNESS_BUFFER_MS = 5 * 60 * 1000;
  const allTimestampsMs: number[] = [
    ...fxSubmissions.map((s) => new Date(s.submitted_at).getTime()),
    ...Object.values(fxComments).flatMap((tree) =>
      Array.from(flattenComments(tree), ([c]) => new Date(c.submitted_at).getTime()),
    ),
  ];
  const maxFixtureMs = Math.max(...allTimestampsMs);
  const targetMaxMs = Date.now() - FRESHNESS_BUFFER_MS;
  const timeShiftMs = Math.max(0, maxFixtureMs - targetMaxMs);
  if (timeShiftMs > 0) {
    const shiftHours = (timeShiftMs / 3_600_000).toFixed(1);
    console.log(`→ Shifting fixture timestamps back ${shiftHours}h to anchor freshest row at NOW-5min`);
  }
  const shiftDate = (iso: string): Date =>
    new Date(new Date(iso).getTime() - timeShiftMs);

  /* ── Submissions ───────────────────────────────────────────── */
  console.log(`→ Inserting ${fxSubmissions.length} submissions…`);
  for (const s of fxSubmissions) {
    const id = submissionIdByFixtureId.get(s.id)!;
    const authorId = userIdByUsername.get(s.user);
    if (!authorId) {
      console.warn(`  skip submission ${s.id}: unknown author "${s.user}"`);
      continue;
    }
    const submittedAt = shiftDate(s.submitted_at);
    const score = s.upvotes - s.downvotes;
    await db.insert(submissions).values({
      id,
      authorId,
      type: s.type,
      title: s.title,
      url: s.url ?? null,
      text: s.text ?? null,
      state: s.state ?? "approved",
      score,
      readingTimeMin: s.reading_time_min ?? null,
      podcastMeta: s.podcast_meta ?? null,
      toolMeta: s.tool_meta ?? null,
      createdAt: submittedAt,
      publishedAt: (s.state ?? "approved") === "approved" ? submittedAt : null,
    });
  }

  /* ── submission_tags ───────────────────────────────────────── */
  let tagLinkCount = 0;
  for (const s of fxSubmissions) {
    const submissionId = submissionIdByFixtureId.get(s.id)!;
    for (const tagSlug of s.tags) {
      await db.insert(submissionTags).values({ submissionId, tagSlug });
      tagLinkCount++;
    }
  }
  console.log(`  → ${tagLinkCount} submission_tags links`);

  /* ── Comments (flat insert; trigger karma firing on score=0 is a no-op) */
  let commentCount = 0;
  for (const [submissionFixtureId, tree] of Object.entries(fxComments)) {
    const submissionId = submissionIdByFixtureId.get(submissionFixtureId);
    if (!submissionId) continue;
    for (const [c, parentFixtureId] of flattenComments(tree)) {
      const id = commentIdByFixtureId.get(c.id)!;
      const authorId = userIdByUsername.get(c.user);
      if (!authorId) continue;
      const parentId = parentFixtureId
        ? commentIdByFixtureId.get(parentFixtureId) ?? null
        : null;
      await db.insert(comments).values({
        id,
        authorId,
        submissionId,
        parentId,
        body: c.body,
        state: c.state ?? "approved",
        score: c.upvotes - c.downvotes,
        createdAt: shiftDate(c.submitted_at),
      });
      commentCount++;
    }
  }
  console.log(`→ Inserted ${commentCount} comments across the tree`);

  /* ── Projects (minimal — owner_id required by FK) ──────────── */
  console.log(`→ Inserting ${fxProjects.length} projects…`);
  // Pick the first user as default owner if fixture doesn't specify one.
  const defaultOwnerId = userIdByUsername.values().next().value as string;
  for (const p of fxProjects) {
    const ownerId = p.owner
      ? userIdByUsername.get(p.owner) ?? defaultOwnerId
      : defaultOwnerId;
    await db.insert(projects).values({
      slug: p.slug,
      name: p.name,
      blurb: p.tagline ?? null,
      ownerId,
    });
  }

  console.log("✓ seed complete");
}

seed().catch((err) => {
  console.error("✗ seed failed:", err);
  process.exit(1);
});
