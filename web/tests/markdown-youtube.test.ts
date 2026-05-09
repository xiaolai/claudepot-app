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
    { allowMediaEmbeds: true },
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
    { allowMediaEmbeds: true },
  );
  check("directive: iframe present", html.includes("<iframe"));
  check("directive: nocookie src", html.includes(`youtube-nocookie.com/embed/${ID}`));
}

// 4. Hostile hand-rolled iframe is dropped even with allowYoutube=true.
{
  const html = await renderMarkdown(
    `<iframe src="https://evil.com/xss"></iframe>`,
    { allowMediaEmbeds: true },
  );
  check("hostile iframe: dropped", !html.includes("evil.com"));
  check("hostile iframe: no iframe tag", !html.includes("<iframe"));
}

// 5. Hostile iframe pretending to be a watch URL is dropped.
{
  const html = await renderMarkdown(
    `<iframe src="https://www.youtube.com/embed/${ID}"></iframe>`,
    { allowMediaEmbeds: true },
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
    { allowMediaEmbeds: true },
  );
  check("inline URL: no iframe", !html.includes("<iframe"));
  check("inline URL: anchor present", html.includes("<a href"));
}

// 7. Hand-rolled iframe with autoplay= is dropped (query string
//    rejected by the tightened regex).
{
  const html = await renderMarkdown(
    `<iframe src="https://www.youtube-nocookie.com/embed/${ID}?autoplay=1"></iframe>`,
    { allowMediaEmbeds: true },
  );
  check("autoplay query: dropped", !html.includes("<iframe"));
}

// 8. Hand-rolled iframe with the canonical SRC but missing
//    sandbox/referrerpolicy gets the safe attrs RE-STAMPED, not
//    omitted (defense in depth on the sanitize side).
{
  const html = await renderMarkdown(
    `<iframe src="https://www.youtube-nocookie.com/embed/${ID}"></iframe>`,
    { allowMediaEmbeds: true },
  );
  check("hand-rolled valid iframe: kept", html.includes("<iframe"));
  check("hand-rolled valid iframe: sandbox restamped", html.includes("sandbox="));
  check("hand-rolled valid iframe: referrerpolicy restamped", html.includes("referrerpolicy="));
  check("hand-rolled valid iframe: lazy restamped", html.includes("loading=\"lazy\""));
}

// 9. URL inside a fenced code block is NOT embedded (pre-pass skips
//    fence content). The code block itself should still render as code.
{
  const md = "Intro.\n\n```\nhttps://youtu.be/" + ID + "\n```\n\nAfter.";
  const html = await renderMarkdown(md, { allowMediaEmbeds: true });
  check("fenced URL: no iframe", !html.includes("<iframe"));
  check("fenced URL: stays in code block", html.includes("<pre") || html.includes("<code"));
}

console.log("");
console.log(`${passed} passed, ${failed} failed`);
if (failed > 0) process.exit(1);
