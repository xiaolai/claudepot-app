#!/usr/bin/env node
// Backfill importer: foxed JSONL → ClauDepot fixtures.
// Usage:
//   node design/scripts/import-backfill.mjs --dry      → write .candidates.json
//   node design/scripts/import-backfill.mjs --write    → mutate fixtures
//
// Run --dry first; review .candidates.json; then --write.

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  NOW_ISO, WINDOW_DAYS, SEED,
  TITLE_BLOCKLIST, URL_BLOCKLIST, TITLE_DROP_LIST,
  RELEVANCE_REQUIRED_SOURCES, RELEVANCE_REGEX,
  SOURCE_QUOTA, SOURCE_WEIGHT, KEYWORD_BONUS,
  classifyType, TAG_RULES, SOURCE_DEFAULT_TAGS,
  SYSTEM_USER, HUMAN_SUBMITTERS, KARMA_PER_SUBMISSION,
  REJECTION_REASONS, PENDING_REASONS,
  VOICE_TEMPLATES, REPLY_TEMPLATES,
  TAG_DISPLAY, ALT_BANK, RELATED_BANK,
} from "./backfill.config.mjs";

// ── Paths ───────────────────────────────────────────────────────────

const __filename = fileURLToPath(import.meta.url);
const ROOT = path.resolve(path.dirname(__filename), "../..");
const SOURCE_JSONL = "/Users/joker/github/xiaolai/myprojects/foxed/out/backfill.jsonl";
const FIXTURES = path.join(ROOT, "design/fixtures");
const SUBMISSIONS_FILE = path.join(FIXTURES, "submissions.json");
const COMMENTS_FILE = path.join(FIXTURES, "comments.json");
const USERS_FILE = path.join(FIXTURES, "users.json");
const AGENTS_FILE = path.join(FIXTURES, "agents.json");
const CANDIDATES_FILE = path.join(ROOT, "design/scripts/.candidates.json");

// ── CLI ─────────────────────────────────────────────────────────────

const args = new Set(process.argv.slice(2));
const DRY = args.has("--dry") || (!args.has("--write"));
const WRITE = args.has("--write");

// ── Seeded PRNG (mulberry32) ────────────────────────────────────────

function mulberry32(seed) {
  let s = seed >>> 0;
  return function () {
    s = (s + 0x6D2B79F5) >>> 0;
    let t = s;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}
const rng = mulberry32(SEED);
const rint = (lo, hi) => Math.floor(rng() * (hi - lo + 1)) + lo;
const pick = (arr) => arr[Math.floor(rng() * arr.length)];

// ── Helpers ─────────────────────────────────────────────────────────

function readJsonl(file) {
  return fs.readFileSync(file, "utf8")
    .split("\n")
    .filter((l) => l.trim())
    .map((l) => JSON.parse(l));
}

function readJson(file) {
  return JSON.parse(fs.readFileSync(file, "utf8"));
}

function writeJson(file, data) {
  fs.writeFileSync(file, JSON.stringify(data, null, 2) + "\n");
}

function domainOf(url) {
  if (!url) return "claudepot.com";
  try {
    const host = new URL(url).hostname.replace(/^www\./, "");
    if (host === "old.reddit.com") return "reddit.com";
    return host;
  } catch {
    return "unknown";
  }
}

// ── Pipeline: load + filter ─────────────────────────────────────────

function loadAndFilter() {
  const raw = readJsonl(SOURCE_JSONL);
  const kept = [];
  let dropped = 0;
  for (const r of raw) {
    if (!r.url || !r.title) { dropped++; continue; }
    if (TITLE_BLOCKLIST.some((re) => re.test(r.title))) { dropped++; continue; }
    if (TITLE_DROP_LIST.some((re) => re.test(r.title))) { dropped++; continue; }
    if (URL_BLOCKLIST.some((re) => re.test(r.url))) { dropped++; continue; }
    if ((SOURCE_QUOTA[r.source] ?? -1) === 0) { dropped++; continue; }
    if (!(r.source in SOURCE_QUOTA)) { dropped++; continue; }
    // Topical relevance gate for noisy sources
    if (RELEVANCE_REQUIRED_SOURCES.has(r.source)) {
      const text = `${r.title} ${r.summary || ""}`;
      if (!RELEVANCE_REGEX.test(text)) { dropped++; continue; }
    }
    kept.push(r);
  }
  return { kept, dropped };
}

// ── Pipeline: score ─────────────────────────────────────────────────

function scoreItem(item) {
  let s = SOURCE_WEIGHT[item.source] ?? 0;
  const text = `${item.title} ${item.summary || ""}`;
  for (const [re, bonus] of KEYWORD_BONUS) {
    if (re.test(text)) s += bonus;
  }
  // HN score field, log-scaled, when present
  if (typeof item.score === "number" && item.score > 0) {
    s += Math.log2(item.score + 1);
  }
  return s;
}

// ── Pipeline: per-source top-N ──────────────────────────────────────

function applyQuota(scored) {
  const bySource = new Map();
  for (const x of scored) {
    if (!bySource.has(x.source)) bySource.set(x.source, []);
    bySource.get(x.source).push(x);
  }
  const out = [];
  for (const [source, list] of bySource) {
    list.sort((a, b) => b._score - a._score);
    const quota = SOURCE_QUOTA[source] ?? 0;
    out.push(...list.slice(0, quota));
  }
  return out;
}

// ── Pipeline: tag classification ────────────────────────────────────

function classifyTags(item) {
  const tags = new Set(SOURCE_DEFAULT_TAGS[item.source] ?? []);
  const text = `${item.title} ${item.summary || ""}`;
  for (const [tag, re] of TAG_RULES) {
    if (re.test(text)) tags.add(tag);
  }
  // Cap at 3, prioritizing most specific (in TAG_RULES order = most specific first)
  const ordered = [];
  // Source defaults first (they're authoritative)
  for (const t of (SOURCE_DEFAULT_TAGS[item.source] ?? [])) ordered.push(t);
  for (const [tag] of TAG_RULES) {
    if (tags.has(tag) && !ordered.includes(tag)) ordered.push(tag);
  }
  return ordered.slice(0, 3);
}

// ── Pipeline: timestamp distribution ────────────────────────────────

const NOW_MS = new Date(NOW_ISO).getTime();
const DAY_MS = 86_400_000;

// Weighted: 60% in last 7 days, 25% days 8-14, 15% days 15-28
function pickTimestamp() {
  const r = rng();
  let days;
  if (r < 0.60) days = rng() * 7;
  else if (r < 0.85) days = 7 + rng() * 7;
  else days = 14 + rng() * (WINDOW_DAYS - 14);
  // Bias toward business hours (9-14 UTC)
  const baseMs = NOW_MS - days * DAY_MS;
  const date = new Date(baseMs);
  date.setUTCHours(9 + Math.floor(rng() * 8)); // 9-17 UTC
  date.setUTCMinutes(Math.floor(rng() * 60));
  date.setUTCSeconds(Math.floor(rng() * 60));
  // Don't return future timestamps
  if (date.getTime() > NOW_MS) return new Date(NOW_MS - rng() * 3600_000).toISOString();
  return date.toISOString();
}

// ── Pipeline: vote shape ────────────────────────────────────────────

// Power-law: most posts modest, some big.
function voteShape(item) {
  const r = rng();
  let up;
  if (r < 0.05) up = 300 + Math.floor(rng() * 350); // top 5%
  else if (r < 0.20) up = 120 + Math.floor(rng() * 180); // next 15%
  else if (r < 0.50) up = 40 + Math.floor(rng() * 80);
  else up = 8 + Math.floor(rng() * 35);
  // Boost for high-score items
  if (item._score >= 25) up = Math.max(up, 200 + Math.floor(rng() * 200));
  if (item._score >= 35) up = Math.max(up, 350 + Math.floor(rng() * 300));
  // Downvotes: small fraction of upvotes
  const down = Math.floor(up * (0.005 + rng() * 0.04));
  return { upvotes: up, downvotes: down };
}

// ── Pipeline: type meta ─────────────────────────────────────────────

const LANG_BY_REPO = {
  "openai/codex": "Rust",
  "ggml-org/llama.cpp": "C++",
  "ollama/ollama": "Go",
  "smithery-ai/cli": "TypeScript",
  "langchain-ai/langgraph": "Python",
  "Kilo-Org/kilocode": "TypeScript",
  "modelcontextprotocol/registry": "Go",
  "shinpr/claude-code-workflows": "TypeScript",
  "lobehub/lobe-chat": "TypeScript",
  "mem0ai/mem0": "Python",
  "chroma-core/chroma": "Python",
};

function inferRepo(url) {
  const m = url.match(/github\.com\/([^/]+\/[^/]+)/);
  return m ? m[1] : null;
}

function inferLanguage(url, source) {
  const repo = inferRepo(url);
  if (repo && LANG_BY_REPO[repo]) return LANG_BY_REPO[repo];
  if (source === "github_releases_coding") return pick(["Rust", "TypeScript", "Go", "Python"]);
  if (source === "github_releases_mcp") return pick(["TypeScript", "Python", "Go"]);
  if (source === "github_releases_agents") return pick(["Python", "TypeScript"]);
  return pick(["TypeScript", "Python", "Rust", "Go"]);
}

function makeToolMeta(item) {
  const stars = (() => {
    const m = item.summary?.match(/\[(\d+)★/);
    if (m) return parseInt(m[1]);
    const r = rng();
    if (r < 0.1) return 5000 + Math.floor(rng() * 50000);
    if (r < 0.4) return 500 + Math.floor(rng() * 4000);
    return 30 + Math.floor(rng() * 400);
  })();
  return {
    stars,
    language: inferLanguage(item.url, item.source),
    last_commit_relative: pick(["today", "yesterday", "2 days ago", "3 days ago", "5 days ago", "1 week ago"]),
  };
}

function makePodcastMeta(item) {
  return {
    duration_min: rint(15, 180),
    host: item.author || pick(["Lex Fridman", "swyx", "Ben & David", "Lenny", "Latent Space"]),
  };
}

function readingTime(item) {
  const len = (item.summary?.length ?? 0) + item.title.length;
  return Math.max(3, Math.min(25, Math.round(len / 80)));
}

// ── Pipeline: user assignment ───────────────────────────────────────

let humanCursor = 0;
function nextHuman() {
  const u = HUMAN_SUBMITTERS[humanCursor % HUMAN_SUBMITTERS.length];
  humanCursor++;
  return u;
}

function assignUser(item) {
  // Auto-posts: anthropic news + all github releases + github activity stream
  if (
    item.source === "anthropic_news" ||
    item.source === "github_activity" ||
    item.source.startsWith("github_releases")
  ) {
    return { user: SYSTEM_USER, auto_posted: true };
  }
  return { user: nextHuman(), auto_posted: false };
}

// ── Pipeline: title rewrites for low-quality sources ────────────────

function rewriteTitle(item) {
  if (item.source === "github_activity" && item.extras?.gh_user) {
    const ev = item.extras.gh_event;
    const target = item.extras.gh_target_repo;
    const evWord = {
      pr_merged: "merged a PR into",
      pr_opened: "opened a PR on",
      issue_opened: "filed an issue on",
      pushed: "pushed to",
    }[ev] ?? "contributed to";
    return `${item.extras.gh_user} ${evWord} ${target}`;
  }
  return item.title;
}

// ── Pipeline: build candidate ───────────────────────────────────────

let nextId = 100;
function buildCandidate(item) {
  const title = rewriteTitle(item);
  const type = classifyType(item);
  const tags = classifyTags(item);
  const { user, auto_posted } = assignUser(item);
  const submitted_at = pickTimestamp();
  const { upvotes, downvotes } = voteShape(item);

  const c = {
    id: String(nextId++),
    user,
    type,
    tags,
    title,
    url: item.url,
    domain: domainOf(item.url),
    subjects: [],
    upvotes,
    downvotes,
    comments: 0, // updated after comment generation
    submitted_at,
  };

  if (auto_posted) c.auto_posted = true;

  if (type === "tool" && /github\.com/.test(item.url)) c.tool_meta = makeToolMeta(item);
  if (type === "podcast") c.podcast_meta = makePodcastMeta(item);
  if (type === "tutorial" || type === "article") c.reading_time_min = readingTime(item);

  // For Ask HN / discussion items, include summary as text body
  if (type === "discussion" && item.summary) {
    c.text = item.summary.slice(0, 600);
  }

  // Internal annotations (stripped on write)
  c._source = item.source;
  c._score = item._score;
  c._summary = item.summary?.slice(0, 200);

  return c;
}

// ── Pipeline: moderation states ─────────────────────────────────────

function applyModerationStates(candidates) {
  // Moderation pool: only items from sources where rejected/pending makes sense.
  // Auto-posted GH activity / releases / Anthropic news are never moderated.
  const moderatable = candidates.filter((c) =>
    !c.auto_posted &&
    !["github_activity", "anthropic_news"].includes(c._source) &&
    !c._source.startsWith("github_releases"),
  );
  const sorted = [...moderatable].sort((a, b) => a._score - b._score);
  const rejected = sorted.slice(0, 6);
  const pending = sorted.slice(6, 12);
  for (let i = 0; i < rejected.length; i++) {
    const c = rejected[i];
    const reason = REJECTION_REASONS[i % REJECTION_REASONS.length];
    c.state = "rejected";
    c.upvotes = 0;
    c.downvotes = 0;
    c.ai_decision = {
      reason: reason.reason,
      confidence: reason.confidence,
      tags_assigned: [],
      decided_at: c.submitted_at,
    };
  }
  for (let i = 0; i < pending.length; i++) {
    const c = pending[i];
    const reason = PENDING_REASONS[i % PENDING_REASONS.length];
    c.state = "pending";
    c.upvotes = 0;
    c.downvotes = 0;
    c.ai_decision = {
      reason: reason.reason,
      confidence: reason.confidence,
      tags_assigned: c.tags,
      type_assigned: c.type,
      decided_at: c.submitted_at,
    };
  }
}

// ── Pipeline: hot score (matches loader) ────────────────────────────

function hotScore(c) {
  const ageHours = (NOW_MS - new Date(c.submitted_at).getTime()) / 3_600_000;
  const net = c.upvotes - c.downvotes;
  return Math.max(net - 1, 0) / Math.pow(ageHours + 2, 1.8);
}

// ── Pipeline: comment generation ────────────────────────────────────

function templateFill(template, post) {
  const tagSlug = post.tags[0] ?? "claude-code";
  const tagName = TAG_DISPLAY[tagSlug] ?? tagSlug;
  return template
    .replace(/\{TAG\}/g, tagName)
    .replace(/\{ALT\}/g, pick(ALT_BANK))
    .replace(/\{RELATED\}/g, pick(RELATED_BANK));
}

function pickCommenter(post, agents, used) {
  // Filter by tag overlap, exclude already-used in this thread
  const candidates = agents.filter((a) =>
    a.interest_tags.some((t) => post.tags.includes(t)) && !used.has(a.username)
  );
  if (candidates.length === 0) {
    const fallback = agents.filter((a) => !used.has(a.username));
    return pick(fallback);
  }
  return pick(candidates);
}

function commentTimestamp(post, prior) {
  const postMs = new Date(post.submitted_at).getTime();
  const minMs = prior ? new Date(prior).getTime() + 60_000 : postMs + 5 * 60_000;
  // Exponential delay, max 48h after post
  const maxMs = Math.min(postMs + 48 * 3600_000, NOW_MS - 60_000);
  if (minMs >= maxMs) return new Date(maxMs).toISOString();
  const span = maxMs - minMs;
  const r = Math.pow(rng(), 2); // skew early
  return new Date(minMs + r * span).toISOString();
}

let nextCommentId = 1000;
function makeComment(post, commenter, body, parentTs) {
  const ts = commentTimestamp(post, parentTs);
  return {
    id: `c${post.id}-${nextCommentId++}`,
    user: commenter.username,
    submitted_at: ts,
    upvotes: rint(0, 25),
    downvotes: rng() < 0.15 ? rint(0, 3) : 0,
    body,
    children: [],
  };
}

function templatedThread(post, agents) {
  // Skip rejected posts entirely; pending get 0-1 comments.
  if (post.state === "rejected") return [];
  const isPending = post.state === "pending";
  // Comment count: 0-2 typical, 0 sometimes, weighted by hot score
  const r = rng();
  let n;
  if (isPending) n = r < 0.7 ? 0 : 1;
  else if (r < 0.20) n = 0;
  else if (r < 0.65) n = 1;
  else if (r < 0.92) n = 2;
  else n = 3;

  const used = new Set();
  const top = [];
  for (let i = 0; i < n; i++) {
    const commenter = pickCommenter(post, agents, used);
    if (!commenter) break;
    used.add(commenter.username);
    const tmpl = pick(VOICE_TEMPLATES[commenter.voice] ?? VOICE_TEMPLATES.terse);
    const body = templateFill(tmpl, post);
    const c = makeComment(post, commenter, body, post.submitted_at);

    // 1-in-5 chance of a single reply
    if (rng() < 0.20) {
      const replier = pickCommenter(post, agents, used);
      if (replier) {
        used.add(replier.username);
        const replyTmpl = pick(REPLY_TEMPLATES[replier.voice] ?? REPLY_TEMPLATES.terse);
        const replyBody = templateFill(replyTmpl, post);
        c.children.push(makeComment(post, replier, replyBody, c.submitted_at));
      }
    }
    top.push(c);
  }
  return top;
}

// ── Summary printer ─────────────────────────────────────────────────

function summarize(candidates) {
  const bySource = {};
  const byType = {};
  const byTag = {};
  let pending = 0, rejected = 0;
  let earliest = NOW_ISO, latest = "1970-01-01";
  for (const c of candidates) {
    bySource[c._source] = (bySource[c._source] ?? 0) + 1;
    byType[c.type] = (byType[c.type] ?? 0) + 1;
    for (const t of c.tags) byTag[t] = (byTag[t] ?? 0) + 1;
    if (c.state === "pending") pending++;
    if (c.state === "rejected") rejected++;
    if (c.submitted_at < earliest) earliest = c.submitted_at;
    if (c.submitted_at > latest) latest = c.submitted_at;
  }
  console.log("\n=== backfill candidates ===");
  console.log(`Total: ${candidates.length}`);
  console.log(`Time range: ${earliest.slice(0, 10)} → ${latest.slice(0, 10)}`);
  console.log(`Pending: ${pending}   Rejected: ${rejected}\n`);

  console.log("By source:");
  for (const [k, v] of Object.entries(bySource).sort((a, b) => b[1] - a[1])) {
    console.log(`  ${k.padEnd(28)} ${v}`);
  }
  console.log("\nBy type:");
  for (const [k, v] of Object.entries(byType).sort((a, b) => b[1] - a[1])) {
    console.log(`  ${k.padEnd(12)} ${v}`);
  }
  console.log("\nBy tag:");
  for (const [k, v] of Object.entries(byTag).sort((a, b) => b[1] - a[1])) {
    console.log(`  ${k.padEnd(20)} ${v}`);
  }
  console.log("\nTop 12 by score:");
  const top = [...candidates].sort((a, b) => b._score - a._score).slice(0, 12);
  for (const c of top) {
    const t = c.title.length > 70 ? c.title.slice(0, 67) + "..." : c.title;
    console.log(`  [${c._score.toFixed(1).padStart(5)}] ${c.user.padEnd(11)} ${c.type.padEnd(9)} ${t}`);
  }
  console.log("\nLowest 6 (will become rejected):");
  const low = [...candidates].sort((a, b) => a._score - b._score).slice(0, 6);
  for (const c of low) {
    const t = c.title.length > 60 ? c.title.slice(0, 57) + "..." : c.title;
    console.log(`  [${c._score.toFixed(1).padStart(5)}] ${t}`);
  }
}

// ── Strip internals before write ────────────────────────────────────

function stripInternal(c) {
  const { _source, _score, _summary, ...rest } = c;
  return rest;
}

// ── Main ────────────────────────────────────────────────────────────

function main() {
  const { kept, dropped } = loadAndFilter();
  console.log(`Loaded ${kept.length + dropped} entries; kept ${kept.length}, dropped ${dropped}.`);

  const scored = kept.map((item) => ({ ...item, _score: scoreItem(item) }));
  const quota = applyQuota(scored);
  console.log(`After quotas: ${quota.length} candidates.`);

  // Build candidates (assigns IDs, timestamps, votes, meta)
  const candidates = quota.map(buildCandidate);
  // Post-build: moderation states (zeros out votes for pending/rejected)
  applyModerationStates(candidates);

  // Compute hot ranking and identify top 40 (gets hand-crafted threads later)
  const ranked = [...candidates].sort((a, b) => hotScore(b) - hotScore(a));
  const TOP_N_HANDCRAFT = 40;
  const handcraftIds = new Set(
    ranked.filter((c) => c.state !== "rejected" && c.state !== "pending").slice(0, TOP_N_HANDCRAFT).map((c) => c.id),
  );

  // Generate templated comments for non-handcraft posts
  const agents = readJson(AGENTS_FILE);
  const commentMap = {};
  for (const c of candidates) {
    if (handcraftIds.has(c.id)) {
      commentMap[c.id] = []; // placeholder for hand-crafting
    } else {
      commentMap[c.id] = templatedThread(c, agents);
    }
    // Update comment count on submission
    c.comments = countComments(commentMap[c.id]);
  }

  summarize(candidates);
  console.log(`\nHand-craft slots reserved for top ${handcraftIds.size} posts (no templated comments).`);
  console.log(`Templated comments: ${Object.values(commentMap).reduce((a, b) => a + countComments(b), 0)}`);

  if (DRY) {
    writeJson(CANDIDATES_FILE, {
      candidates,
      commentMap,
      handcraftIds: [...handcraftIds],
      meta: { generated_at: new Date().toISOString(), now_iso: NOW_ISO },
    });
    console.log(`\nDry-run output written to ${path.relative(ROOT, CANDIDATES_FILE)}.`);
    console.log(`Re-run with --write to apply to fixtures.`);
    return;
  }

  if (WRITE) {
    applyToFixtures(candidates, commentMap);
    console.log(`\nFixtures updated. Top-${handcraftIds.size} comment threads remain empty for hand-crafting.`);
  }
}

function countComments(nodes) {
  let n = 0;
  for (const c of nodes) n += 1 + countComments(c.children);
  return n;
}

function applyToFixtures(candidates, commentMap) {
  // Submissions
  const existingSubs = readJson(SUBMISSIONS_FILE);
  const newSubs = candidates.map(stripInternal);
  writeJson(SUBMISSIONS_FILE, [...existingSubs, ...newSubs]);

  // Comments
  const existingComments = readJson(COMMENTS_FILE);
  const merged = { ...existingComments };
  for (const [id, threads] of Object.entries(commentMap)) {
    if (threads.length > 0) merged[id] = threads;
  }
  writeJson(COMMENTS_FILE, merged);

  // Karma bumps
  const users = readJson(USERS_FILE);
  const counts = {};
  for (const c of candidates) {
    counts[c.user] = (counts[c.user] ?? 0) + 1;
  }
  for (const u of users) {
    if (counts[u.username] && !u.is_system) {
      u.karma += counts[u.username] * KARMA_PER_SUBMISSION;
    }
  }
  writeJson(USERS_FILE, users);
}

main();
