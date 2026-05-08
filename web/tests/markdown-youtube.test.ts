/**
 * Integration test for the YouTube embed path through the full
 * renderMarkdown pipeline (pre-pass → marked → sanitize-html →
 * decoration). Complements tests/youtube-embed.test.ts which only
 * covers the pre-pass in isolation.
 *
 * Run with:  pnpm tsx tests/markdown-youtube.test.ts
 */

import { renderMarkdown } from "../src/lib/markdown";

let passed = 0;
let failed = 0;

function check(label: string, predicate: boolean) {
  if (predicate) {
    console.log(`PASS  ${label}`);
    passed += 1;
  } else {
    console.error(`FAIL  ${label}`);
    failed += 1;
  }
}

const ID = "dQw4w9WgXcQ";

// 1. Default mode — URL stays as a plain anchor, no iframe.
{
  const html = await renderMarkdown(
    `Intro.\n\nhttps://www.youtube.com/watch?v=${ID}\n\nAfter.`,
  );
  check("default: no iframe", !html.includes("<iframe"));
  check("default: anchor present", html.includes("<a href=\"https://www.youtube.com/watch"));
}

// 2. allowYoutube=true with bare URL — iframe survives sanitize.
{
  const html = await renderMarkdown(
    `Intro.\n\nhttps://www.youtube.com/watch?v=${ID}\n\nAfter.`,
    { allowYoutube: true },
  );
  check("opt-in: iframe present", html.includes("<iframe"));
  check("opt-in: src is nocookie", html.includes(`youtube-nocookie.com/embed/${ID}`));
  check("opt-in: src is NOT youtube.com/embed", !/src="https:\/\/www\.youtube\.com\/embed/.test(html));
  check("opt-in: sandbox attr present", html.includes("sandbox="));
  check("opt-in: referrerpolicy attr present", html.includes("referrerpolicy="));
  check("opt-in: lazy-loaded", html.includes("loading=\"lazy\""));
  check("opt-in: wrapper class present", html.includes("class=\"proto-yt-embed\""));
}

// 3. Directive shortcode.
{
  const html = await renderMarkdown(
    `Intro.\n\n:youtube[${ID}]\n\nAfter.`,
    { allowYoutube: true },
  );
  check("directive: iframe present", html.includes("<iframe"));
  check("directive: nocookie src", html.includes(`youtube-nocookie.com/embed/${ID}`));
}

// 4. Hostile hand-rolled iframe is dropped even with allowYoutube=true.
{
  const html = await renderMarkdown(
    `<iframe src="https://evil.com/xss"></iframe>`,
    { allowYoutube: true },
  );
  check("hostile iframe: dropped", !html.includes("evil.com"));
  check("hostile iframe: no iframe tag", !html.includes("<iframe"));
}

// 5. Hostile iframe pretending to be a watch URL is dropped.
{
  const html = await renderMarkdown(
    `<iframe src="https://www.youtube.com/embed/${ID}"></iframe>`,
    { allowYoutube: true },
  );
  // NOTE: sanitize re-validates against the nocookie regex; even though
  // youtube.com/embed/ID is "real", we accept ONLY the nocookie origin
  // so user-pasted iframes can't bypass our pre-pass and skip privacy.
  check("youtube.com iframe: dropped", !html.includes("<iframe"));
}

// 6. Inline URL inside a sentence is left alone (no embed).
{
  const html = await renderMarkdown(
    `Watch this https://youtu.be/${ID} now.`,
    { allowYoutube: true },
  );
  check("inline URL: no iframe", !html.includes("<iframe"));
  check("inline URL: anchor present", html.includes("<a href"));
}

console.log("");
console.log(`${passed} passed, ${failed} failed`);
if (failed > 0) process.exit(1);
