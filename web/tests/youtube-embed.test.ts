/**
 * YouTube embed pre-pass — covers ID extraction across the five
 * canonical URL shapes and the two trigger forms (bare URL, directive
 * shortcode), plus the rejection cases we care about: malformed IDs,
 * non-YouTube hosts, garbage URLs, prose-on-the-same-line URLs.
 *
 * Run with:  pnpm tsx tests/youtube-embed.test.ts
 */

import assert from "node:assert/strict";
import {
  extractYoutubeId,
  rewriteYoutubeEmbeds,
} from "../src/lib/youtube-embed";

let passed = 0;
let failed = 0;

function check<T>(label: string, actual: T, expected: T) {
  try {
    assert.deepEqual(actual, expected);
    console.log(`PASS  ${label}`);
    passed += 1;
  } catch {
    console.error(`FAIL  ${label}`);
    console.error(`      got      ${JSON.stringify(actual)}`);
    console.error(`      expected ${JSON.stringify(expected)}`);
    failed += 1;
  }
}

function checkContains(label: string, haystack: string, needle: string) {
  if (haystack.includes(needle)) {
    console.log(`PASS  ${label}`);
    passed += 1;
  } else {
    console.error(`FAIL  ${label}`);
    console.error(`      "${needle}" not found in:`);
    console.error(`      ${haystack.replace(/\n/g, "\\n")}`);
    failed += 1;
  }
}

const ID = "dQw4w9WgXcQ"; // canonical 11-char ID

// ── extractYoutubeId — accept matrix ─────────────────────────────
check("watch", extractYoutubeId(`https://www.youtube.com/watch?v=${ID}`), ID);
check("watch m.", extractYoutubeId(`https://m.youtube.com/watch?v=${ID}`), ID);
check("watch + extra params", extractYoutubeId(`https://www.youtube.com/watch?v=${ID}&t=42s&list=PL1`), ID);
check("youtu.be", extractYoutubeId(`https://youtu.be/${ID}`), ID);
check("youtu.be + ?t=", extractYoutubeId(`https://youtu.be/${ID}?t=10`), ID);
check("shorts", extractYoutubeId(`https://www.youtube.com/shorts/${ID}`), ID);
check("live", extractYoutubeId(`https://www.youtube.com/live/${ID}`), ID);
check("embed", extractYoutubeId(`https://www.youtube.com/embed/${ID}`), ID);
check("nocookie embed", extractYoutubeId(`https://www.youtube-nocookie.com/embed/${ID}`), ID);
check("naked youtube.com (no www)", extractYoutubeId(`https://youtube.com/watch?v=${ID}`), ID);

// ── extractYoutubeId — reject matrix ─────────────────────────────
check("garbage", extractYoutubeId("not a url"), null);
check("non-yt host", extractYoutubeId("https://vimeo.com/12345"), null);
check("watch missing v=", extractYoutubeId("https://www.youtube.com/watch"), null);
check("watch with empty v=", extractYoutubeId("https://www.youtube.com/watch?v="), null);
check("watch ID too short", extractYoutubeId("https://www.youtube.com/watch?v=abc123"), null);
check("watch ID too long", extractYoutubeId(`https://www.youtube.com/watch?v=${ID}EXTRA`), null);
check("youtu.be ID too short", extractYoutubeId("https://youtu.be/abc"), null);
check("path-traversal in shorts", extractYoutubeId("https://www.youtube.com/shorts/../evil"), null);
check("javascript: scheme", extractYoutubeId(`javascript:alert(${ID})`), null);
check("data: scheme", extractYoutubeId("data:text/html,<script>"), null);

// ── rewriteYoutubeEmbeds — bare URL paragraph ───────────────────
const bareWatch = rewriteYoutubeEmbeds(
  `Some intro paragraph.\n\nhttps://www.youtube.com/watch?v=${ID}\n\nAfter.`,
);
checkContains("bare watch URL → iframe", bareWatch, `youtube-nocookie.com/embed/${ID}`);
checkContains("bare watch URL → wrapper", bareWatch, `class="proto-yt-embed"`);

const bareShort = rewriteYoutubeEmbeds(`https://youtu.be/${ID}`);
checkContains("bare youtu.be → iframe", bareShort, `youtube-nocookie.com/embed/${ID}`);

const bareShorts = rewriteYoutubeEmbeds(`https://www.youtube.com/shorts/${ID}`);
checkContains("bare /shorts/ → iframe", bareShorts, `youtube-nocookie.com/embed/${ID}`);

// Same-line prose disqualifies — Reddit/Discourse rule.
const inline = rewriteYoutubeEmbeds(
  `Watch this: https://www.youtube.com/watch?v=${ID} it's good.`,
);
check("prose-on-same-line URL → not embedded", inline.includes("<iframe"), false);

// Non-YouTube URL on its own line → untouched.
const otherUrl = rewriteYoutubeEmbeds("\nhttps://example.com/article\n");
check("non-yt bare URL → not embedded", otherUrl.includes("<iframe"), false);

// ── rewriteYoutubeEmbeds — directive shortcode ───────────────────
const directive = rewriteYoutubeEmbeds(`Some intro.\n\n:youtube[${ID}]\n\nAfter.`);
checkContains("directive → iframe", directive, `youtube-nocookie.com/embed/${ID}`);

const directiveBadId = rewriteYoutubeEmbeds(":youtube[short]");
check("directive with bad ID → untouched", directiveBadId.includes("<iframe"), false);

// Inline directive (mid-paragraph) — by design, only paragraph-alone
// directives are rewritten. This keeps :youtube[…] inside running
// prose from breaking out into a block.
const inlineDirective = rewriteYoutubeEmbeds(`See :youtube[${ID}] above.`);
check("inline directive → not embedded", inlineDirective.includes("<iframe"), false);

// ── rewriteYoutubeEmbeds — output security shape ─────────────────
const sample = rewriteYoutubeEmbeds(`https://youtu.be/${ID}`);
checkContains("iframe carries sandbox", sample, "sandbox=");
checkContains("iframe carries referrerpolicy", sample, "referrerpolicy=");
checkContains("iframe is lazy-loaded", sample, `loading="lazy"`);
checkContains("iframe uses youtube-nocookie", sample, "youtube-nocookie.com");
check("iframe NEVER points at youtube.com directly",
  /src="https:\/\/www\.youtube\.com/.test(sample),
  false);

// ── rewriteYoutubeEmbeds — idempotency ───────────────────────────
const once = rewriteYoutubeEmbeds(`https://youtu.be/${ID}`);
const twice = rewriteYoutubeEmbeds(once);
check("rewrite is idempotent", once, twice);

// ── rewriteYoutubeEmbeds — fenced code blocks ────────────────────
// Backtick fence — URL inside must NOT be rewritten.
const fenced = rewriteYoutubeEmbeds(
  "Intro.\n\n\`\`\`\nhttps://youtu.be/" + ID + "\n\`\`\`\n\nAfter.",
);
check("URL inside ``` fence → not embedded", fenced.includes("<iframe"), false);
checkContains("URL inside ``` fence → preserved", fenced, `youtu.be/${ID}`);

// Tilde fence — same protection.
const fencedTilde = rewriteYoutubeEmbeds(
  `\n~~~\nhttps://youtu.be/${ID}\n~~~\n`,
);
check("URL inside ~~~ fence → not embedded", fencedTilde.includes("<iframe"), false);

// Directive shortcode inside a fence is also preserved verbatim.
const fencedDirective = rewriteYoutubeEmbeds(
  "\`\`\`\n:youtube[" + ID + "]\n\`\`\`",
);
check("directive inside fence → not embedded", fencedDirective.includes("<iframe"), false);
checkContains("directive inside fence → preserved", fencedDirective, `:youtube[${ID}]`);

// Outside the fence still works after a fenced block.
const mixedFence = rewriteYoutubeEmbeds(
  "\`\`\`\nhttps://youtu.be/" + ID + "\n\`\`\`\n\nhttps://youtu.be/" + ID + "\n",
);
checkContains("URL after fence → embedded", mixedFence, "<iframe");
check(
  "URL inside fence stays bare URL",
  /\`\`\`\nhttps:\/\/youtu\.be\//.test(mixedFence),
  true,
);

// Indented code (4+ spaces) — the 0-3-space gate keeps the rewriter
// out of indented blocks the same way CommonMark does.
const indented = rewriteYoutubeEmbeds(`    https://youtu.be/${ID}\n`);
check("indented (4-space) URL → not embedded", indented.includes("<iframe"), false);

console.log("");
console.log(`${passed} passed, ${failed} failed`);
if (failed > 0) process.exit(1);
