/**
 * Integration test for Spotify + Apple Podcasts embed paths through
 * the full renderMarkdown pipeline (pre-pass → marked → sanitize-html
 * → decoration). Mirrors tests/markdown-youtube.test.ts; together
 * they cover all three platforms wired into `allowMediaEmbeds`.
 *
 * Run with:  pnpm tsx tests/markdown-media-embeds.test.ts
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

const SPOTIFY_EPISODE = "4rOoJ6Egrf8K2IrywzwOMk";
const SPOTIFY_SHOW = "2MAi0BvDc6GTFvKFPXnkCL";
const APPLE_SHOW =
  "https://podcasts.apple.com/us/podcast/the-tim-ferriss-show/id863897795";
const APPLE_EPISODE = `${APPLE_SHOW}?i=1000635467219`;

/* ── Default mode — no iframes ───────────────────────────────── */

{
  const html = await renderMarkdown(
    `Intro.\n\nhttps://open.spotify.com/episode/${SPOTIFY_EPISODE}\n\nAfter.`,
  );
  check("default: Spotify URL stays as anchor, no iframe", !html.includes("<iframe"));
  check(
    "default: Spotify anchor present",
    html.includes(`<a href="https://open.spotify.com/episode/${SPOTIFY_EPISODE}"`),
  );
}

{
  const html = await renderMarkdown(`Intro.\n\n${APPLE_SHOW}\n\nAfter.`);
  check("default: Apple URL stays as anchor, no iframe", !html.includes("<iframe"));
}

/* ── allowMediaEmbeds=true ──────────────────────────────────── */

{
  const html = await renderMarkdown(
    `Intro.\n\nhttps://open.spotify.com/episode/${SPOTIFY_EPISODE}\n\nAfter.`,
    { allowMediaEmbeds: true },
  );
  check("Spotify episode: iframe present", html.includes("<iframe"));
  check(
    "Spotify episode: src points at embed/episode/<id>",
    html.includes(
      `src="https://open.spotify.com/embed/episode/${SPOTIFY_EPISODE}"`,
    ),
  );
  check(
    "Spotify episode: sandbox attr enforced post-sanitize",
    html.includes(`sandbox="allow-scripts allow-same-origin allow-popups"`),
  );
  check(
    "Spotify episode: wrapper class survives sanitize",
    html.includes(`class="proto-spotify-embed"`),
  );
  check(
    "Spotify episode: lazy-load enforced",
    html.includes(`loading="lazy"`),
  );
}

{
  const html = await renderMarkdown(
    `Intro.\n\nhttps://open.spotify.com/show/${SPOTIFY_SHOW}\n\nAfter.`,
    { allowMediaEmbeds: true },
  );
  check(
    "Spotify show: src points at embed/show/<id>",
    html.includes(`src="https://open.spotify.com/embed/show/${SPOTIFY_SHOW}"`),
  );
}

{
  const html = await renderMarkdown(`Intro.\n\n${APPLE_SHOW}\n\nAfter.`, {
    allowMediaEmbeds: true,
  });
  check("Apple show: iframe present", html.includes("<iframe"));
  check(
    "Apple show: src points at embed.podcasts.apple.com",
    html.includes(
      `src="https://embed.podcasts.apple.com/us/podcast/the-tim-ferriss-show/id863897795"`,
    ),
  );
  check(
    "Apple show: wrapper class survives sanitize",
    html.includes(`class="proto-applepod-embed"`),
  );
}

{
  const html = await renderMarkdown(`Intro.\n\n${APPLE_EPISODE}\n\nAfter.`, {
    allowMediaEmbeds: true,
  });
  check(
    "Apple episode: src carries ?i=<episode-id>",
    html.includes(
      `src="https://embed.podcasts.apple.com/us/podcast/the-tim-ferriss-show/id863897795?i=1000635467219"`,
    ),
  );
}

/* ── Defense in depth — sanitizer drops malformed iframes ───── */

{
  // Hand-rolled iframe with right host but wrong path shape — sanitize
  // must drop it because the SRC regex is exact-match.
  const hostile = `<iframe src="https://open.spotify.com/embed/episode/../../malicious"></iframe>\n`;
  const html = await renderMarkdown(hostile, { allowMediaEmbeds: true });
  check(
    "hostile Spotify iframe with path traversal: dropped",
    !html.includes("<iframe"),
  );
}

{
  // Hostile Apple iframe with extra query param besides the allowed `i=`.
  const hostile = `<iframe src="https://embed.podcasts.apple.com/us/podcast/foo/id123?i=456&inject=1"></iframe>\n`;
  const html = await renderMarkdown(hostile, { allowMediaEmbeds: true });
  check(
    "hostile Apple iframe with extra query: dropped",
    !html.includes("<iframe"),
  );
}

{
  // Non-platform iframe always gets dropped.
  const hostile = `<iframe src="https://evil.example.com/payload"></iframe>\n`;
  const html = await renderMarkdown(hostile, { allowMediaEmbeds: true });
  check("non-platform iframe: dropped", !html.includes("<iframe"));
}

/* ── Backward-compat: allowYoutube alias still works ────────── */

{
  const html = await renderMarkdown(
    `Intro.\n\nhttps://open.spotify.com/episode/${SPOTIFY_EPISODE}\n\nAfter.`,
    { allowYoutube: true },
  );
  check(
    "allowYoutube=true now also allows Spotify (alias)",
    html.includes("<iframe"),
  );
}

console.log(`\n${passed} passed, ${failed} failed`);
process.exit(failed > 0 ? 1 : 0);
