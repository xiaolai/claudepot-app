// Backfill configuration: filter rules, scoring, classification, templates.
// Pure data. Logic lives in import-backfill.mjs.

export const NOW_ISO = "2026-04-30T18:00:00Z";
export const WINDOW_DAYS = 28;
export const SEED = 0xC1A0DEB07; // deterministic RNG

// ── Hard blocks: dropped before scoring ─────────────────────────────

export const TITLE_BLOCKLIST = [
  /\bis claude (down|broken|out|having)\b/i,
  /\bclaude\.ai (down|unavailable|outage)/i,
  /\brate[- ]limited\b/i,
  /\blost my (chat|conversation|message)\b/i,
  /\b(lol|lmao|🤣|🥲|🥹|😂)\b/,
  /^(thanks for the advice|when you've got|how nosy|ok dude|that's me and|i vibe-coded gta)/i,
  /\bdrop your best\b/i,
  /\bme and claud\b/i,
  /\bclaude reset limits\b/i,
];

export const URL_BLOCKLIST = [
  /[?&](ref|aff|affiliate|partner)=/i,
  /\/affiliate\//i,
];

// Sources requiring topical relevance — drop unless title/summary mentions
// Claude / AI / agents / coding-tools / models. Keeps off-topic HN+reddit out.
export const RELEVANCE_REQUIRED_SOURCES = new Set([
  "hn", "hn_algolia", "reddit_localllama", "reddit_ml",
]);

export const RELEVANCE_REGEX = /\b(claude|anthropic|opus|haiku|sonnet|gpt|gemini|llama|qwen|codex|cursor|mcp|agent|llm|prompt|inference|rag|retrieval|embedding|fine[- ]?tun|jailbreak|copilot|aider|openai|deepseek|mistral|whisper|chatgpt|kilocode|langchain|langgraph|smithery|ollama|chroma|hugging[- ]?face|transformer|tokeniz|prompt[- ]injection|model[- ]release|reasoning model)/i;

// Explicit drop list — titles that the relevance regex catches but are off-topic
export const TITLE_DROP_LIST = [
  /^biology is a burrito/i,
  /^craig venter/i,
  /^joby kicks off/i,
  /^monad tutorials/i,
  /^where the goblins/i,
  /^functional programmers/i,
  /^cursor camp/i,
  /^copy fail/i,
  /^a grounded conceptual/i,
  /^noctua releases/i,
  /\bcooling fans\b/i,
  /\b(jax|cfd|aerojax)\b/i,
  /^icml \d/i,
  /^aerojax/i,
  /^ramp's sheets/i,
];

// ── Source quotas: cap per source after scoring ─────────────────────

export const SOURCE_QUOTA = {
  anthropic_news: 2,
  github_releases_official: 7,
  github_releases_mcp: 7,
  github_releases_coding: 3,
  github_releases_agents: 18,
  github_releases_frontier: 2,
  github_releases_chat: 7,
  github_releases_community: 4,
  github_releases_infra: 4,
  simonw: 25,
  hn_algolia: 70,
  hn: 12,
  reddit_claudeai: 18,
  reddit_cursor: 6,
  reddit_localllama: 6,
  reddit_ml: 4,
  youtube_ai: 5,
  openai_blog: 4,
  github_activity: 12,
  // Mainstream press + off-topic reddit: drop entirely.
  hbr_ai: 0,
  nyt_ai: 0,
  economist_scitech: 0,
  wired_ai: 0,
  newyorker_business: 0,
  wapo_tech: 0,
  reddit_chatgpt: 0,
  reddit_googlegemini: 0,
  reddit_openai: 0,
};

// ── Scoring weights ─────────────────────────────────────────────────

export const SOURCE_WEIGHT = {
  anthropic_news: 20,
  simonw: 12,
  github_releases_official: 8,
  github_releases_mcp: 8,
  github_releases_coding: 8,
  github_releases_agents: 8,
  github_releases_frontier: 8,
  github_releases_chat: 5,
  github_releases_community: 5,
  github_releases_infra: 5,
  hn_algolia: 3,
  hn: 3,
  openai_blog: 2,
  reddit_claudeai: 3,
  reddit_cursor: 2,
  reddit_localllama: 2,
  reddit_ml: 2,
  youtube_ai: 4,
  github_activity: 2,
};

// Keyword bonuses on title (case-insensitive). Cumulative.
export const KEYWORD_BONUS = [
  [/\bclaude\b/i, 5],
  [/\banthropic\b/i, 5],
  [/\b(opus|haiku|sonnet)\s*\d/i, 6],
  [/\bmcp\b/i, 4],
  [/\bagent(s|ic)?\b/i, 3],
  [/\bcodex\b/i, 2],
  [/\bcursor\b/i, 2],
  [/\bprompt\b/i, 2],
  [/\bcache|caching\b/i, 2],
  [/\b1m\b|\b1[- ]million\b/i, 3],
  [/^show hn:/i, 2],
  [/^ask hn:/i, 1],
  [/\b(eval|benchmark)\b/i, 2],
  [/\bmodel context protocol\b/i, 4],
  [/\b(jailbreak|prompt[- ]injection|zero[- ]day)\b/i, 2],
];

// ── Type classification (first match wins) ──────────────────────────

export function classifyType(item) {
  const t = item.title || "";
  const u = item.url || "";

  // YouTube → podcast
  if (/youtube\.com|youtu\.be/.test(u)) return "podcast";

  // GitHub release tag → tool
  if (/github\.com\/[^/]+\/[^/]+\/releases\/tag\//.test(u)) return "tool";

  // Show HN: → tool (something they built)
  if (/^show hn:/i.test(t)) return "tool";

  // Ask HN: → discussion
  if (/^ask hn:/i.test(t)) return "discussion";

  // simonw "Quoting X" → article
  if (/^quoting /i.test(t) && item.source === "simonw") return "article";

  // GitHub activity → discussion (people-watch)
  if (item.source === "github_activity") return "discussion";

  // Tutorial-ish prefixes
  if (/^(how to|how i|building|setting up|getting started|writing|implementing)/i.test(t))
    return "tutorial";

  // Course-ish
  if (/\b(course|bootcamp|cohort|workshop|class)\b/i.test(t)) return "course";

  // Interview-ish
  if (/\b(interview|ep\.\s*\d|episode \d)\b/i.test(t)) return "interview";

  // Tip-ish (short imperative)
  if (
    /^(use|pin|always|stop|don't|never|prefer|cache|stream)/i.test(t) &&
    t.length < 70
  )
    return "tip";

  // simonw long-form → article
  if (item.source === "simonw" && (item.summary?.length ?? 0) > 200) return "article";

  // Anthropic / OpenAI blog announcements → news
  if (/^(introducing |announcing )/i.test(t)) return "news";

  // GitHub release-y → tool
  if (/github\.com/.test(u) && !/issues|pull|discussion/.test(u)) return "tool";

  // Default
  return "news";
}

// ── Tag classification (regex on title + summary; cap at 3 tags) ────

export const TAG_RULES = [
  ["mcp", /\bmcp\b|\bmodel[- ]context[- ]protocol\b/i],
  ["model-release", /\b(opus|haiku|sonnet|gpt[- ]?[0-9]|gemini[- ]?[0-9]|claude[- ]?[0-9])\b|\bintroducing claude\b|\b(launch|launches|ships|introduced)\b.*\b(model|opus|haiku|sonnet)\b/i],
  ["safety", /\b(safety|jailbreak|alignment|prompt[- ]injection|red[- ]team|zero[- ]day|deletes .* database|rogue|anti[- ]llm)\b/i],
  ["voice", /\b(voice|speech|audio|whisper|vibevoice|hands[- ]free|tts|stt)\b/i],
  ["claude-code", /\b(claude[- ]?code|claude code|cli|cursor|codex|aider|vscode|copilot|coding agent)\b/i],
  ["agents", /\bagent(s|ic)?\b|\b(planner|orchestrat|tool[- ]calling|autonomous|hand[- ]off)\b/i],
  ["long-context", /\b1m\b|\b1[- ]million\b|\blong[- ]context\b|\bcontext[- ]window\b|\b(needle|haystack|rag|retrieval|embedding)\b/i],
  ["prompt-caching", /\bcach(e|ing)\b|\bkv[- ]cache\b|\bprompt[- ]cache\b/i],
  ["evals", /\b(eval|benchmark|regression|swe[- ]bench|leaderboard|harness)\b/i],
  ["infra", /\b(inference|latency|gpu|tensor|throughput|rate[- ]limit|backoff|scaling|deploy|kubernetes|cost)\b/i],
];

// Source-driven default tags (in addition to regex matches).
export const SOURCE_DEFAULT_TAGS = {
  anthropic_news: ["model-release"],
  github_releases_official: ["release-watch"],
  github_releases_mcp: ["release-watch", "mcp"],
  github_releases_coding: ["release-watch", "claude-code"],
  github_releases_agents: ["release-watch", "agents"],
  github_releases_frontier: ["release-watch", "model-release"],
  github_releases_chat: ["release-watch"],
  github_releases_community: ["release-watch"],
  github_releases_infra: ["release-watch", "infra"],
};

// ── Submitter pool ──────────────────────────────────────────────────

// Auto-posted (release-watch + model-release) goes to system account.
export const SYSTEM_USER = "ClauDepot";

// Curated picks rotate through these humans.
export const HUMAN_SUBMITTERS = [
  "ada", "kai", "miro", "lin", "sasha", "zed", "nova", "ren", "ish", "lixiaolai",
];

// ── Karma bumps (per submission) ────────────────────────────────────

export const KARMA_PER_SUBMISSION = 12; // generous; reflects upvote earnings

// ── AI rejection-reason bank (for the 6 'rejected' fixtures) ────────

export const REJECTION_REASONS = [
  { reason: "Title pattern matches promotional spam (\"10x your X with this one tool\"). No substantive content in URL preview.", confidence: 0.92 },
  { reason: "Affiliate/referral link in URL without disclosure. Rule 4 violation.", confidence: 0.94 },
  { reason: "Duplicate — same URL submitted by another user 6 hours earlier.", confidence: 0.97 },
  { reason: "Off-topic. URL preview is about general productivity, no Claude / AI tooling angle.", confidence: 0.85 },
  { reason: "Low-quality content farm domain. Heuristic match against known SEO doorway sites.", confidence: 0.88 },
  { reason: "Title misleadingly clickbait — preview content does not match the claim.", confidence: 0.86 },
];

export const PENDING_REASONS = [
  { reason: "Topic relevance borderline — post is about general LLM ops, not Claude-specific.", confidence: 0.68 },
  { reason: "Self-promotion signal moderate — author has 3 prior posts to the same domain this week.", confidence: 0.62 },
  { reason: "Quality score in middle band. Substantive but no clear novelty over existing posts.", confidence: 0.71 },
  { reason: "Mixed-language content; classifier confidence drops on the non-English passages.", confidence: 0.65 },
  { reason: "Domain unfamiliar to classifier. URL preview reads as legitimate but no signal history.", confidence: 0.74 },
  { reason: "Edge of safety policy — discusses jailbreak techniques. Could be educational or not.", confidence: 0.59 },
];

// ── Templated comments per voice ────────────────────────────────────

// {TAG} substituted with display-name of the post's first tag.
// {ALT} / {RELATED} substituted from small banks.

export const VOICE_TEMPLATES = {
  terse: [
    "+1. Worth the read.",
    "Saw this. Same conclusion.",
    "Bookmarking.",
    "{TAG} has been on my list. Thanks.",
    "Yep.",
    "Same outcome at our shop.",
    "Solid.",
  ],
  dry: [
    "{TAG} is the silent killer of agent budgets. This is a clean breakdown.",
    "Funny how everyone re-discovers this every six months.",
    "Learned it the hard way last quarter. Pay the cost up front.",
    "The interesting part isn't {TAG} itself; it's what happens when you stop relying on it.",
    "Half-right. The other half is observability — and almost no one does it.",
    "Used to think {TAG} was niche. Now it's table stakes.",
    "This is a known problem with a known fix. The fact that it's still news is the news.",
  ],
  earnest: [
    "Took me three reads, but the {TAG} bit clarified something I'd been stuck on for weeks.",
    "Going to try this on our pipeline next week. Will report back.",
    "First one of these I've seen that doesn't oversell. Appreciate the honesty about failure modes.",
    "Saved. Pointing our junior team at this on Monday.",
    "Good piece. The {TAG} angle is what most write-ups miss.",
    "Long but worth it. The benchmarks justify the read.",
    "Thanks for posting. This kind of write-up is exactly why I lurk here.",
  ],
  contrarian: [
    "Or: don't. We tried this in production and the second-order effects ate the savings.",
    "This works until {TAG} stops mattering, which it will.",
    "Hard disagree. Opposite approach worked for us — small, dumb, replicated.",
    "The numbers don't match what we're seeing. Curious what their workload actually looks like.",
    "Fine for a 10-engineer shop. Doesn't scale.",
    "Counter: this solves the wrong problem. {ALT}",
    "Strongly worded post; weakly supported claims. Show me the eval.",
  ],
  discursive: [
    "Reminds me of when we tried {RELATED} two years ago. Same shape, different layer of the stack. Lessons mostly transferred.",
    "There's a deeper question here about why {TAG} keeps re-appearing. My theory: the abstraction is leaky in a way that isn't obvious until you scale past one team.",
    "I keep going back and forth on this. The framing is correct in steady state but breaks down during ramp-up — and ramp-up is where most of us live.",
    "Worth pairing with the {RELATED} discussion from a few weeks back. They're not contradictory but they optimize for different things.",
    "Three thoughts: (1) the {TAG} side is solved; (2) the deployment side isn't; (3) most teams pretend (1) is the bottleneck because (2) is harder to talk about.",
    "On rereading, I think the post understates the cost of switching. We've eaten that cost; six weeks to recover throughput.",
  ],
};

// Reply templates (when a top-level comment gets a child).
export const REPLY_TEMPLATES = {
  terse: ["This.", "Same here.", "+1", "Confirmed.", "Yep, same."],
  dry: [
    "The opposite has also been true at certain scales. Both can be right.",
    "Counterpoint: {ALT}.",
    "Sometimes. Until it isn't.",
  ],
  earnest: [
    "Genuinely useful comment, thanks.",
    "Could you say more about the {TAG} part? That's the bit I'm stuck on.",
    "This matches our data too.",
  ],
  contrarian: [
    "Disagree on the framing. {TAG} isn't the bottleneck.",
    "Numbers, please.",
    "Or you could just not.",
  ],
  discursive: [
    "Adding to this — the same pattern shows up in {RELATED} but with different second-order effects, which is why people get confused when comparing notes across teams.",
    "Right, and the corollary is that the architecture choice cascades into the eval strategy, which is rarely discussed in the same breath.",
  ],
};

// {TAG} substitution targets — display names per slug.
export const TAG_DISPLAY = {
  "mcp": "MCP",
  "agents": "agent orchestration",
  "long-context": "long context",
  "prompt-caching": "prompt caching",
  "claude-code": "Claude Code",
  "evals": "evals",
  "infra": "the infra layer",
  "release-watch": "release-watch",
  "model-release": "model launches",
  "voice": "voice agents",
  "safety": "safety policy",
};

export const ALT_BANK = [
  "the planner-worker pattern, with a 200-line orchestrator",
  "fewer tools, sharper schemas",
  "skip the framework, write the loop",
  "actually just measure it before optimizing",
];

export const RELATED_BANK = [
  "the 'one big agent vs. many small' thread",
  "the long-running-agent cost piece from a few weeks ago",
  "Latent Space's deep dive on Claude memory",
  "the eval harness write-up",
];
