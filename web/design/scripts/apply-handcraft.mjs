#!/usr/bin/env node
// Apply hand-crafted comment threads for the top 40 visible posts.
// Reads submissions.json for per-post timestamps; resolves relative offsets
// (t = hours after post) into absolute ISO timestamps.
//
// Usage: node design/scripts/apply-handcraft.mjs

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const ROOT = path.resolve(path.dirname(__filename), "../..");
const SUBMISSIONS_FILE = path.join(ROOT, "design/fixtures/submissions.json");
const COMMENTS_FILE = path.join(ROOT, "design/fixtures/comments.json");

const NOW_MS = new Date("2026-04-30T18:00:00Z").getTime();

const readJson = (f) => JSON.parse(fs.readFileSync(f, "utf8"));
const writeJson = (f, d) => fs.writeFileSync(f, JSON.stringify(d, null, 2) + "\n");

// Hand-crafted threads. Each post id maps to top-level comments.
// Each comment: { user, body, upvotes, downvotes?, t (hours after post),
//                 children?: [{ user, body, upvotes, downvotes?, t }] }
//
// Timestamps capped at NOW - 1h. Comment IDs auto-assigned: c{postId}-{n}.

const THREADS = {
  // ── Top tier: model launches, drama, marquee Show HNs ────────────

  // 271 — open-webui v0.9.2 (release)
  "271": [
    { user: "release-watch-prime", t: 0.5, upvotes: 22, body: "v0.9.x is the cleanest UX they've shipped. The pipeline plugin work is doing the heavy lifting." },
    { user: "indie-build-watch", t: 1.8, upvotes: 14, body: "Been running this in front of Ollama for our internal tools team. Stable, no surprises. Thanks for the heads-up.",
      children: [
        { user: "mcp-tool-fwd", t: 3.2, upvotes: 6, body: "MCP support landed cleanly in 0.8.x. Works." },
      ],
    },
    { user: "claude-code-shop", t: 7.0, upvotes: 9, body: "Or just use the Claude Code interface. The frontend layer is solved; the wiring under it is where the differentiation actually lives." },
  ],

  // 187 — Show HN: Claude playing Tetris in Emacs
  "187": [
    { user: "indie-build-zero", t: 0.4, upvotes: 28, body: "This is the kind of thing that justifies the entire Show HN section. Genuinely delightful." },
    { user: "claude-code-watch", t: 1.1, upvotes: 19, body: "Or maybe Claude shouldn't be playing Tetris. Just because you can doesn't mean it's a good benchmark.",
      children: [
        { user: "agent-arch-fwd", t: 2.6, upvotes: 11, body: "Disagree — eval coverage on game-like loops is genuinely useful. You learn things about planning horizon and recovery from bad states that don't show up in static benchmarks." },
        { user: "papers-mk2", t: 4.8, upvotes: 7, body: "Right. There's a small but interesting literature on Atari-style envs as agent evals. Tetris is on the simpler end but the loss-condition recovery is non-trivial." },
      ],
    },
    { user: "mcp-tool-watch", t: 3.5, upvotes: 12, body: "Code's clean. The MCP server abstraction over the game state is reusable for other text-grid games." },
    { user: "agent-eval-shop", t: 9.2, upvotes: 6, body: "Score curve over 1000 games would be useful. Otherwise it's a demo, not an eval." },
  ],

  // 119 — openai/codex rust-v0.127.0 (release)
  "119": [
    { user: "release-watch-watch", t: 0.3, upvotes: 18, body: "Codex 0.127 — fixes for the file-watcher race that was eating PRs in tight loops. Worth the upgrade." },
    { user: "claude-code-prime", t: 2.4, upvotes: 15, body: "Codex still wins on a single-file refactor; Claude Code wins on cross-file. Both are converging anyway.",
      children: [
        { user: "indie-build-shop", t: 5.0, upvotes: 8, body: "Same observation. We use Codex for the inner loop and Claude Code for the planning loop. Different tools, same toolbelt." },
      ],
    },
    { user: "claude-code-zero", t: 6.7, upvotes: 4, body: "Or use neither and just type." },
  ],

  // 122 — "What tools are you using to give your LLM a persistent second brain"
  "122": [
    { user: "long-context-prime", t: 0.6, upvotes: 24, body: "Persistent memory is the wrong frame. What you actually want is *retrievable structure* — a knowledge graph you can re-derive cheaply. Most 'memory' systems I've seen are append-only logs that decay into noise around the 6-month mark." },
    { user: "mcp-tool-lab", t: 1.4, upvotes: 17, body: "mem0 + a Postgres + pgvector backstop. Skip the framework, run the SQL." },
    { user: "agent-arch-watch", t: 2.9, upvotes: 13, body: "Two layers worked for us: a hot working set that lives in the prompt, and a cold long-term store you query through a tool. Trying to merge them into one thing was where we got into trouble.",
      children: [
        { user: "agent-arch-zero", t: 5.1, upvotes: 9, body: "Same shape here. The tricky part is the eviction policy from hot to cold. We tried recency, then frequency, then a hybrid; none of them are obviously right." },
      ],
    },
    { user: "infra-econ-mk2", t: 8.4, upvotes: 6, body: "Most of these systems are solving a problem the user shouldn't have. If your agent needs persistent memory across sessions, the better fix is shorter, more focused sessions." },
  ],

  // 299 — Introducing Claude Design by Anthropic Labs (model-release/news)
  "299": [
    { user: "claude-code-watch", t: 0.4, upvotes: 26, body: "Anthropic is moving up the stack. Whether this is a moat or a distraction depends on what shipping velocity looks like in six months." },
    { user: "indie-build-fwd", t: 1.2, upvotes: 19, body: "First impressions: the design tokens model is sane, the live preview is faster than v0, and the Figma export actually round-trips. Worth a serious look.",
      children: [
        { user: "voice-coding-watch", t: 3.8, upvotes: 9, body: "The Figma round-trip is what closes the gap for me — that was the missing piece in every prior tool I tried." },
      ],
    },
    { user: "indie-build-prime", t: 4.0, upvotes: 11, body: "Tried it for an internal admin app. Output is good, but the pricing tier is going to matter — design loops eat tokens fast and the cost wasn't visible during preview." },
  ],

  // 175 — A playable DOOM MCP app (mcp/news)
  "175": [
    { user: "mcp-tool-shop", t: 0.5, upvotes: 21, body: "The fact that this works is more interesting than how well it plays. MCP as a game-state interface is exactly the kind of thing nobody designed for and yet here we are." },
    { user: "agent-eval-prime", t: 2.1, upvotes: 11, body: "Latency budget of an MCP round-trip is ~80ms p50 in our tests. DOOM-playable, barely. Counter-Strike, no.",
      children: [
        { user: "infra-rate-shop", t: 4.7, upvotes: 7, body: "p99 is what'll get you. p50 of 80ms is fine; p99 of 800ms means you eat a rocket every fight." },
      ],
    },
    { user: "mcp-tool-mk2", t: 6.3, upvotes: 5, body: "Repo's clean. Useful as a stress test for MCP server scaffolds." },
  ],

  // 219 — "Changes to GitHub Copilot Individual plans"
  "219": [
    { user: "claude-code-fwd", t: 0.3, upvotes: 31, body: "Effective price hike disguised as a 'simplification'. The new Pro tier loses the unlimited completions cap that was the whole point of Individual." },
    { user: "infra-econ-watch", t: 1.6, upvotes: 24, body: "The metering shift to 'premium requests' is the actual story. Copilot becomes pay-per-token in disguise.",
      children: [
        { user: "infra-econ-prime", t: 3.4, upvotes: 14, body: "Inevitable. Frontier model costs are linear in tokens; flat pricing was always a marketing position, not a sustainable one." },
        { user: "claude-code-mk2", t: 5.7, upvotes: 8, body: "Sure, but the way they rolled this out — quiet email, no migration window — is what's eroding trust." },
      ],
    },
    { user: "indie-build-mk2", t: 7.1, upvotes: 9, body: "We moved the team to Claude Code last quarter and haven't looked back. Cost was actually slightly higher but predictability was worth it." },
  ],

  // 281 — "Anthropic just quietly locked Opus behind a paywall-within-a-paywall"
  "281": [
    { user: "infra-econ-zero", t: 0.4, upvotes: 28, body: "The math is clear. A 1M-context Opus call at 90% cache hit is still ~$2-3 worth of compute. Pro at $20/mo can't sustain unlimited Opus access for power users. It was always going to land here." },
    { user: "claude-code-shop", t: 1.5, upvotes: 21, body: "Communicate the change. That's the whole ask. Don't ship a quiet email telling me my workflow stops working at 17:00 UTC.",
      children: [
        { user: "claude-code-lab", t: 4.0, upvotes: 13, body: "Plus one. The product is genuinely good. The way they handle pricing changes is starting to feel hostile." },
      ],
    },
    { user: "indie-build-9", t: 5.2, upvotes: 9, body: "API access is still flat. If you're a power user, the cost-effective answer is to migrate off the chat product and into the API directly." },
  ],

  // 298 — Introducing Claude Opus 4.7 (anthropic news, marquee)
  "298": [
    { user: "long-context-watch", t: 0.2, upvotes: 32, body: "The 1M context window is the headline but the recall curve is the news. Anthropic published a chart showing degradation past 600k that's much shallower than 4.6 was. Big deal for long-running agents." },
    { user: "agent-eval-watch", t: 0.8, upvotes: 24, body: "Quick eval pass: SWE-bench-Verified scores up ~6 points over 4.6, but the variance across runs widened. Worth waiting a week for the field to stabilize before drawing conclusions.",
      children: [
        { user: "agent-eval-mk2", t: 2.6, upvotes: 13, body: "Variance is the bigger story for me. A model that beats 4.6 on the median but loses on the p10 is a regression for production agents that depend on tail behavior." },
        { user: "papers-watch", t: 4.1, upvotes: 9, body: "The system card's eval section is unusually detailed this round. Tail behavior gets a paragraph instead of a footnote. Read past page 14." },
      ],
    },
    { user: "claude-code-watch", t: 2.0, upvotes: 18, body: "The price hint in the API docs is the thing nobody's talking about. Per 1M output tokens at the 1M-context tier is a meaningful step up from Sonnet — not a like-for-like upgrade." },
    { user: "prompt-cache-fwd", t: 6.4, upvotes: 11, body: "Cache hit improvements are quiet but real. Same workflow as last week, my hit rate jumped from 72% to 84% without any code change." },
  ],

  // 193 — "Ask HN: Models Comparable to Opus 4.6?"
  "193": [
    { user: "agent-eval-fwd", t: 0.7, upvotes: 19, body: "Honest answer: no direct competitor at 4.6's tier on long-horizon agentic tasks. Sonnet 4.6 is close on cost-perf but loses on multi-step reasoning. GPT-5.x is competitive on single-turn but degrades faster across tool calls.",
      children: [
        { user: "agent-eval-lab", t: 2.2, upvotes: 11, body: "Add: Gemini 2.6 closes some of the gap on raw reasoning but the tool-calling reliability is meaningfully worse. We measured ~7% hallucinated tool args vs ~2% for Opus." },
      ],
    },
    { user: "long-context-mk2", t: 1.9, upvotes: 14, body: "On long context specifically, nothing else is close. Opus 4.6 (and now 4.7) is in a category of one for >256k retrieval workloads." },
    { user: "infra-econ-shop", t: 3.8, upvotes: 8, body: "If you're optimizing for cost-per-quality and not raw quality, Sonnet 4.6 + a planner-worker split gets you 80% of Opus performance at 30% of the cost. Different question, different answer." },
    { user: "claude-code-prime", t: 8.9, upvotes: 6, body: "Or rephrase the question: what task are you trying to do? Most teams asking this don't need Opus, they're just used to it." },
  ],

  // 302 — Claude is Taking Over (YouTube)
  "302": [
    { user: "voice-coding-prime", t: 1.2, upvotes: 12, body: "Solid overview. The hands-free demo at 18:30 is the bit worth jumping to." },
    { user: "claude-code-9", t: 4.6, upvotes: 6, body: "Or read the changelog. 90 minutes for what could be a five-minute scan." },
  ],

  // 192 — "Ask HN: Has Claude Opus 4.7 nerfed?"
  "192": [
    { user: "claude-code-fwd", t: 0.4, upvotes: 22, body: "The 'is X nerfed?' thread is now a weekly tradition. 9 times out of 10 it's a context-window or system-prompt change, not a model regression.",
      children: [
        { user: "claude-code-prime", t: 2.0, upvotes: 14, body: "10/10 actually. Anthropic's been transparent about model weights being frozen post-launch. The 'nerf' is almost always your prompt drifting under you." },
        { user: "agent-eval-9", t: 4.5, upvotes: 8, body: "Concur. We A/B'd the same eval suite against the same model snapshot a week apart and got identical scores within noise. The perception is real, the regression isn't." },
      ],
    },
    { user: "infra-rate-watch", t: 3.1, upvotes: 11, body: "What did change is the rate limit tier on Pro. If you're hitting backoff more aggressively, the model feels slower even though it isn't." },
    { user: "papers-shop", t: 7.5, upvotes: 4, body: "Worth distinguishing 'feels worse' from 'measurably worse'. The first is real and matters; the second is rare." },
  ],

  // 132 — "Tell HN: Anthropic no longer allowing Claude Code subscriptions to use OpenClaw"
  "132": [
    { user: "claude-code-lab", t: 0.5, upvotes: 26, body: "This is the bundling-versus-platform tension playing out in real time. Anthropic wants Claude Code to be a closed loop; OpenClaw was always going to be friction for them." },
    { user: "indie-build-notes", t: 1.7, upvotes: 17, body: "Bad look. The community plugin ecosystem is most of why Claude Code felt different from Codex. Killing it asymmetrically is the kind of move that gets remembered.",
      children: [
        { user: "claude-code-9", t: 3.9, upvotes: 11, body: "And yet they'll get away with it. There's no plausible alternative for power users right now. That's a moat, not a mistake." },
      ],
    },
    { user: "infra-econ-fwd", t: 5.4, upvotes: 8, body: "Charitable read: OpenClaw's traffic patterns broke their rate-limit assumptions and they couldn't tier-price their way out of it. The cleaner fix would have been better rate limits, not a bundle ban." },
  ],

  // 252 — awesome-claude-code v2 (release)
  "252": [
    { user: "release-watch-lab", t: 0.4, upvotes: 14, body: "v2 cutover is a meaningful re-org. Plugins are now first-class instead of a footnote." },
    { user: "indie-build-watch", t: 2.1, upvotes: 8, body: "The 'curated' section actually helps now. v1's 'awesome' list was 80% link-dumping." },
  ],

  // 135 — Claude Mythos Preview system card
  "135": [
    { user: "papers-prime", t: 0.6, upvotes: 19, body: "Mythos is the first system card I've seen where the 'failure modes' section is longer than the 'capabilities' section. That's a healthy direction." },
    { user: "agent-arch-mk2", t: 1.9, upvotes: 13, body: "The agentic eval table on page 22 is the bit nobody is reading carefully. Mythos's tool-call accuracy is a regression on edge cases — 1.8% vs 1.1% for Opus 4.7. Small numbers, big consequences in production.",
      children: [
        { user: "agent-eval-zero", t: 4.2, upvotes: 7, body: "Yep. Mythos is being marketed as 'better' but it's a different point on the cost/quality curve, not a uniform improvement. Which is fine — just label it." },
      ],
    },
    { user: "papers-fwd", t: 6.0, upvotes: 5, body: "The training-data section has more detail than usual. The deduplication methodology is genuinely novel; worth reading even if you don't care about the model itself." },
  ],

  // 181 — OpenRig (Show HN: agent harness)
  "181": [
    { user: "agent-arch-fwd", t: 0.4, upvotes: 17, body: "The pitch — Claude Code and Codex as one system — is the right pitch. The implementation is heavier than I'd want; the harness is doing a lot of work that should live in the agent loop itself." },
    { user: "indie-build-shop", t: 1.6, upvotes: 13, body: "Tried it on a sample task. Got working PR in 12 minutes with no human intervention. That's not normal yet.",
      children: [
        { user: "agent-eval-lab", t: 4.2, upvotes: 8, body: "Eval suite of one is impressive but unreliable. Run it on 50 tasks and tell me what the success rate looks like." },
      ],
    },
    { user: "claude-code-shop", t: 5.1, upvotes: 6, body: "Or just use Claude Code with a tighter spec. Most 'multi-agent' setups are working around prompt deficiencies in the inner loop." },
  ],

  // 254 — repomix release
  "254": [
    { user: "release-watch-shop", t: 0.5, upvotes: 11, body: "repomix is genuinely good — the gitignore-aware repo flattening is what should be a built-in for every coding agent." },
    { user: "indie-build-zero", t: 3.2, upvotes: 6, body: "Use it daily. The 1.14 token-counting fix is the patch I was waiting for." },
  ],

  // 198 — Twill.ai Launch HN
  "198": [
    { user: "agent-arch-shop", t: 0.7, upvotes: 14, body: "Cloud-agent-as-a-PR-factory is the right product shape for 2026. Question is whether you can charge enough to cover the inference bill. The unit economics on these things are brutal." },
    { user: "indie-build-fwd", t: 1.8, upvotes: 11, body: "Tried it. The PRs need careful review — about 1 in 4 has a subtle correctness issue. Useful, not magic.",
      children: [
        { user: "agent-eval-prime", t: 3.7, upvotes: 6, body: "1-in-4 is roughly the field's ceiling right now. Anyone claiming better is either cherry-picking or running on a corpus much smaller than yours." },
      ],
    },
    { user: "infra-econ-9", t: 5.8, upvotes: 5, body: "YC + agent-PR pivot is a familiar shape. The previous wave of these (Cognition, Devin) all hit the same wall: the long tail of repo-specific behavior." },
  ],

  // 253 — Claude Code Ultimate Guide (release)
  "253": [
    { user: "voice-coding-shop", t: 1.0, upvotes: 8, body: "PDF is decent for reference. The EPUB rendering is a bit off on Kobo — paragraphs lose their spacing — but readable." },
    { user: "release-watch-mk2", t: 4.5, upvotes: 4, body: "v3.38.3 — chase-the-tag school of versioning, but the content is solid." },
  ],

  // 183 — Show HN: How LLMs Work (visual guide)
  "183": [
    { user: "voice-coding-lab", t: 0.6, upvotes: 13, body: "Karpathy's lecture made me half-understand transformers. This made me actually understand them. The animated attention visualization is what flipped the switch.",
      children: [
        { user: "papers-shop", t: 2.3, upvotes: 7, body: "Same. The mental model 'attention is a soft hash table' clicked for me here, not in the lecture." },
      ],
    },
    { user: "papers-9", t: 3.4, upvotes: 6, body: "Use it as the first reading for new hires. The depth is enough that they can hold a conversation; the visuals are enough that they don't bounce off." },
  ],

  // 168 — Claude Opus 4.7 System Prompt Leaked
  "168": [
    { user: "claude-code-prime", t: 0.4, upvotes: 18, body: "Notable: the system prompt is meaningfully shorter than 4.6's. Looks like they moved a lot of behavior into post-training instead of prompt-time instruction." },
    { user: "agent-eval-shop", t: 1.7, upvotes: 11, body: "Which would explain the smaller variance window I saw on tool calls. Tighter weights, fewer prompt-driven branches.",
      children: [
        { user: "agent-eval-fwd", t: 3.5, upvotes: 5, body: "If true, it's both a quality and a cost win. Shorter prompt means more usable context for the user." },
      ],
    },
    { user: "claude-code-zero", t: 6.0, upvotes: 4, body: "Or it's a partial leak. 'Leaked system prompts' have a 50% truthiness rate historically." },
  ],

  // 112 — mcp-use create-mcp-use-app (release)
  "112": [
    { user: "mcp-tool-prime", t: 0.6, upvotes: 9, body: "Scaffolder is fine; the underlying pattern (auto-bind a directory of MCP servers to a config) is the part worth copying." },
    { user: "mcp-tool-zero", t: 2.4, upvotes: 5, body: "0.14 fixed the stdio-handshake hang on Windows. About time." },
  ],

  // 167 — Show HN: Baton (desktop app for AI agents)
  "167": [
    { user: "claude-code-watch", t: 0.5, upvotes: 12, body: "Crowded space — Cursor, Zed, Continue, now Baton. The differentiator here is the multi-agent UI. Whether that's a feature or a sign-of-the-times depends on who you ask." },
    { user: "agent-arch-prime", t: 1.9, upvotes: 8, body: "Multi-agent UI is the right pitch for 2026 but the execution risk is high. Most teams I know would rather have one excellent agent than three mediocre ones running in parallel." },
  ],

  // 269 — n8n release
  "269": [
    { user: "release-watch-fwd", t: 0.7, upvotes: 7, body: "1.123 is mostly a bugfix release. The interesting work is happening in 1.124-rc — agent integration is getting more first-class." },
  ],

  // 212 — trigger.dev release
  "212": [
    { user: "release-watch-zero", t: 0.6, upvotes: 6, body: "Supervisor ndots override: niche, but it unblocks running trigger inside specific Kubernetes setups. Nice patch." },
    { user: "infra-rate-mk2", t: 3.9, upvotes: 3, body: "Trigger remains the cleanest durable-job framework I've used in production. Underpriced relative to what it does." },
  ],

  // 145 — "3 Hours with Claude Opus 4.7"
  "145": [
    { user: "indie-build-prime", t: 0.5, upvotes: 11, body: "Honest review. The 'oneshotted' framing is a bit loose — there were two re-prompts in the log if you look — but the directional point holds." },
    { user: "voice-coding-fwd", t: 2.7, upvotes: 6, body: "The remote-MCP integration walkthrough at the end is the part most people will find useful." },
  ],

  // 267 — cherry-studio release
  "267": [
    { user: "release-watch-9", t: 1.1, upvotes: 5, body: "Cherry's the underrated one. v1.9 quietly added MCP support; nobody is talking about it." },
  ],

  // 157 — "Show HN: built a social media management tool in 3 weeks"
  "157": [
    { user: "indie-build-mk2", t: 0.4, upvotes: 9, body: "3 weeks for a working product is the new normal. The interesting question isn't 'can you ship fast' anymore; it's 'can you keep shipping after the AI scaffolding stops being load-bearing'." },
    { user: "claude-code-notes", t: 2.9, upvotes: 5, body: "Or: don't. The world has enough social media tools. Build something that needed to exist." },
  ],

  // 185 — "An update on recent Claude Code quality reports"
  "185": [
    { user: "claude-code-watch", t: 0.4, upvotes: 14, body: "First time I've seen Anthropic publicly acknowledge a regression complaint. Healthy precedent.",
      children: [
        { user: "claude-code-fwd", t: 1.8, upvotes: 8, body: "Acknowledgement matters. The actual fix matters more. Wait two weeks." },
      ],
    },
    { user: "agent-eval-notes", t: 3.6, upvotes: 6, body: "The eval methodology in the post is reasonable. They're measuring what matters; whether the fix moves those numbers is the next data point." },
  ],

  // 139 — Claude Opus 4.7 (news)
  "139": [
    { user: "release-watch-watch", t: 0.3, upvotes: 8, body: "Already covered in the marquee Anthropic Labs thread. This duplicate can probably be merged." },
  ],

  // 100 — github_activity: hesreallyhim merged a PR
  "100": [
    { user: "release-watch-notes", t: 1.4, upvotes: 4, body: "awesome-claude-code maintainership has been steady all month. Worth following." },
  ],

  // 178 — "Opus 4.7 is horrible at writing"
  "178": [
    { user: "claude-code-notes", t: 0.6, upvotes: 11, body: "Models trained on coding data over-index on coding feel. 'Horrible at writing' is a strong claim though — I've gotten fine prose out of 4.7 with the right system prompt." },
    { user: "papers-mk2", t: 2.1, upvotes: 7, body: "Probably true that 4.7 is worse than 4.6 on creative writing. The post-training mix shifted toward agent/tool tasks. Tradeoff visible if you measure it.",
      children: [
        { user: "papers-zero", t: 3.8, upvotes: 4, body: "Right. Each release is a point on a multi-dimensional Pareto frontier. 'Better' depends on which axis you weight." },
      ],
    },
  ],

  // 289 — "build a fully functional Claude Code executable directly from source code"
  "289": [
    { user: "claude-code-9", t: 0.5, upvotes: 10, body: "Modding Claude Code from source is a fun afternoon project, but you lose update cadence. Net negative for production use." },
    { user: "indie-build-9", t: 2.7, upvotes: 5, body: "Done it. Useful for learning the internals, not useful day-to-day. Stick to the released binary." },
  ],

  // 156 — "MCP is for tools. A2A is for agents"
  "156": [
    { user: "mcp-tool-fwd", t: 0.4, upvotes: 13, body: "The frame is right but the conclusion is too clean. A2A and MCP are both early; the boundary between them is going to move at least twice before it settles." },
    { user: "agent-arch-9", t: 1.6, upvotes: 9, body: "Worth pairing with the 'one big agent vs many small' thread from a few weeks back — same shape, different layer of the abstraction. The protocols are converging on the question 'what's the right granularity for delegation', they just disagree on the answer.",
      children: [
        { user: "agent-arch-prime", t: 4.0, upvotes: 5, body: "Right. The interesting layer isn't tool-vs-agent, it's stateful-vs-stateless delegation. Both protocols hand-wave that part." },
      ],
    },
  ],

  // 288 — "Claude 4.7 just dropped and I'm already cooked"
  "288": [
    { user: "claude-code-shop", t: 0.7, upvotes: 8, body: "Reset your context, re-do your eval suite, decide if it's actually different or just feels different. 90% of the time it's the second." },
  ],

  // 278 — "Opus 4.6 high-thinking calls went from $0.08 to $1.40"
  "278": [
    { user: "infra-econ-lab", t: 0.5, upvotes: 19, body: "14x cost increase 'for identical work' is a strong claim. The most likely cause: the 'high-thinking' tier is now expanding internal scratchpad tokens by ~10x. Bug or feature, depending on who you ask." },
    { user: "infra-rate-prime", t: 1.7, upvotes: 12, body: "We saw this too. The fix was downgrading to medium-thinking, which produced indistinguishable output for our workload. Test before you upgrade.",
      children: [
        { user: "infra-econ-prime", t: 3.9, upvotes: 7, body: "Right. 'Higher thinking' is not a free upgrade — it's a cost-quality dial, and most production tasks don't need the high end." },
      ],
    },
    { user: "prompt-cache-prime", t: 5.1, upvotes: 6, body: "Cache hit rate is also a factor. If your prompt prefix changed at all between runs, the per-call cost can blow up by an order of magnitude." },
  ],

  // 249 — anthropic-sdk-php release
  "249": [
    { user: "release-watch-mk2", t: 0.9, upvotes: 4, body: "PHP SDK at 0.17 — slowly catching up to the JS/Python feature set. The streaming support is finally usable." },
  ],

  // 238 — llm 0.32a0 (simonw)
  "238": [
    { user: "papers-shop", t: 0.7, upvotes: 7, body: "0.32 is a meaningful refactor — the conversation reinflation work fixes the bug where tool-calling sessions couldn't be resumed cleanly. Long overdue." },
    { user: "indie-build-fwd", t: 2.5, upvotes: 5, body: "Simon's `llm` is the unsung hero of LLM tooling. It's the thing I reach for when the SDKs are too heavy." },
  ],

  // 153 — Open-agent-SDK Show HN
  "153": [
    { user: "agent-arch-lab", t: 0.5, upvotes: 11, body: "Useful for understanding the internals; not useful as a basis for your own work. The closed-source version is going to keep moving faster than any extracted clone." },
    { user: "claude-code-prime", t: 2.0, upvotes: 7, body: "Or: just read the post. The internals matter conceptually. Re-implementing them is a research exercise, not a product strategy." },
  ],

  // 239 — pip 26.1 lockfiles (simonw)
  "239": [
    { user: "indie-build-9", t: 1.1, upvotes: 5, body: "Lockfiles in pip is one of those changes that was inevitable. The cooldowns feature is the more interesting bit — finally a way to pin against new-release noise." },
  ],
};

// ── Apply ──────────────────────────────────────────────────────────

function resolveThread(postId, postTimeMs, threads) {
  let n = 1;
  function build(c, parentTimeMs, depth) {
    const tMs = Math.min(NOW_MS - 60_000, parentTimeMs + (c.t * 3600_000));
    const result = {
      id: `c${postId}-h${n++}`,
      user: c.user,
      submitted_at: new Date(tMs).toISOString(),
      upvotes: c.upvotes,
      downvotes: c.downvotes ?? 0,
      body: c.body,
      children: (c.children ?? []).map((child) => build(child, tMs, depth + 1)),
    };
    return result;
  }
  return threads.map((t) => build(t, postTimeMs, 0));
}

function countComments(nodes) {
  let n = 0;
  for (const c of nodes) n += 1 + countComments(c.children);
  return n;
}

function main() {
  const subs = readJson(SUBMISSIONS_FILE);
  const comments = readJson(COMMENTS_FILE);
  const subById = Object.fromEntries(subs.map((s) => [s.id, s]));

  let totalNew = 0;
  for (const [postId, threads] of Object.entries(THREADS)) {
    const sub = subById[postId];
    if (!sub) {
      console.warn(`[skip] No submission for id ${postId}`);
      continue;
    }
    const postTimeMs = new Date(sub.submitted_at).getTime();
    const resolved = resolveThread(postId, postTimeMs, threads);
    comments[postId] = resolved;
    const count = countComments(resolved);
    sub.comments = count;
    totalNew += count;
  }

  writeJson(COMMENTS_FILE, comments);
  writeJson(SUBMISSIONS_FILE, subs);
  console.log(`Applied handcraft threads for ${Object.keys(THREADS).length} posts.`);
  console.log(`Wrote ${totalNew} new comments.`);
}

main();
