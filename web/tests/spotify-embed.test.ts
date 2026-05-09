/**
 * Spotify embed pre-pass — covers episode/show ID extraction, locale
 * prefix tolerance, query-string stripping, and rejection of
 * non-podcast Spotify content (tracks, albums, playlists).
 *
 * Run with:  pnpm tsx tests/spotify-embed.test.ts
 */

import assert from "node:assert/strict";
import {
  extractSpotifyMatch,
  rewriteSpotifyEmbeds,
} from "../src/lib/spotify-embed";

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

function checkExcludes(label: string, haystack: string, needle: string) {
  if (!haystack.includes(needle)) {
    console.log(`PASS  ${label}`);
    passed += 1;
  } else {
    console.error(`FAIL  ${label}`);
    console.error(`      "${needle}" UNEXPECTEDLY found in:`);
    console.error(`      ${haystack.replace(/\n/g, "\\n")}`);
    failed += 1;
  }
}

const EPISODE_ID = "4rOoJ6Egrf8K2IrywzwOMk";
const SHOW_ID = "2MAi0BvDc6GTFvKFPXnkCL";

/* ── extractSpotifyMatch — accept paths ───────────────────────── */

check(
  "episode URL",
  extractSpotifyMatch(`https://open.spotify.com/episode/${EPISODE_ID}`),
  { kind: "episode", id: EPISODE_ID },
);
check(
  "show URL",
  extractSpotifyMatch(`https://open.spotify.com/show/${SHOW_ID}`),
  { kind: "show", id: SHOW_ID },
);
check(
  "episode URL with si= tracking param dropped",
  extractSpotifyMatch(
    `https://open.spotify.com/episode/${EPISODE_ID}?si=abcdef1234567890`,
  ),
  { kind: "episode", id: EPISODE_ID },
);
check(
  "episode URL with locale prefix tolerated",
  extractSpotifyMatch(`https://open.spotify.com/intl-ja/episode/${EPISODE_ID}`),
  { kind: "episode", id: EPISODE_ID },
);
check(
  "show URL with trailing slash",
  extractSpotifyMatch(`https://open.spotify.com/show/${SHOW_ID}/`),
  { kind: "show", id: SHOW_ID },
);
check(
  "www.open.spotify.com host stripped",
  extractSpotifyMatch(`https://www.open.spotify.com/episode/${EPISODE_ID}`),
  { kind: "episode", id: EPISODE_ID },
);

/* ── extractSpotifyMatch — reject paths ───────────────────────── */

check(
  "track URL rejected (not a podcast)",
  extractSpotifyMatch(
    "https://open.spotify.com/track/4cOdK2wGLETKBW3PvgPWqT",
  ),
  null,
);
check(
  "album URL rejected",
  extractSpotifyMatch(
    "https://open.spotify.com/album/2up3OPMp9Tb4dAKM2erWXQ",
  ),
  null,
);
check(
  "playlist URL rejected",
  extractSpotifyMatch(
    "https://open.spotify.com/playlist/37i9dQZF1DXcBWIGoYBM5M",
  ),
  null,
);
check(
  "non-spotify host rejected",
  extractSpotifyMatch("https://example.com/episode/" + EPISODE_ID),
  null,
);
check(
  "id with wrong length rejected",
  extractSpotifyMatch("https://open.spotify.com/episode/short"),
  null,
);
check(
  "id with hyphens rejected (Spotify uses base62, no hyphens)",
  extractSpotifyMatch(
    "https://open.spotify.com/episode/4rOoJ6Egrf8K2IrywzwO-k",
  ),
  null,
);
check("garbage URL rejected", extractSpotifyMatch("not-a-url"), null);
check("empty string rejected", extractSpotifyMatch(""), null);

/* ── rewriteSpotifyEmbeds — bare URL paragraph trigger ────────── */

const bareEpisode = rewriteSpotifyEmbeds(
  `Here's the episode:\n\nhttps://open.spotify.com/episode/${EPISODE_ID}\n\nGood listen.`,
);
checkContains(
  "bare episode URL → iframe",
  bareEpisode,
  `<iframe src="https://open.spotify.com/embed/episode/${EPISODE_ID}"`,
);
checkContains(
  "iframe carries title",
  bareEpisode,
  `title="Spotify embed"`,
);
checkContains("iframe is lazy-loaded", bareEpisode, `loading="lazy"`);
checkContains(
  "iframe carries strict referrerpolicy",
  bareEpisode,
  `referrerpolicy="strict-origin-when-cross-origin"`,
);
checkContains(
  "iframe is sandboxed (no allow-same-origin — see embed-attrs.ts)",
  bareEpisode,
  `sandbox="allow-scripts allow-popups"`,
);
checkContains(
  "wrapper class set for CSS hookup",
  bareEpisode,
  `class="proto-spotify-embed"`,
);

const bareShow = rewriteSpotifyEmbeds(
  `https://open.spotify.com/show/${SHOW_ID}`,
);
checkContains(
  "bare show URL → embed/show iframe",
  bareShow,
  `src="https://open.spotify.com/embed/show/${SHOW_ID}"`,
);

/* ── Reject contexts ──────────────────────────────────────────── */

const insideFence = rewriteSpotifyEmbeds(
  `\`\`\`\nhttps://open.spotify.com/episode/${EPISODE_ID}\n\`\`\`\n`,
);
checkExcludes("URL inside fenced code block NOT embedded", insideFence, "<iframe");

const indented = rewriteSpotifyEmbeds(
  `    https://open.spotify.com/episode/${EPISODE_ID}\n`,
);
checkExcludes(
  "URL with 4-space indent (code block) NOT embedded",
  indented,
  "<iframe",
);

const inProse = rewriteSpotifyEmbeds(
  `Listen here: https://open.spotify.com/episode/${EPISODE_ID} — pretty good.`,
);
checkExcludes(
  "URL inside prose (not own line) NOT embedded",
  inProse,
  "<iframe",
);

/* ── Idempotency ──────────────────────────────────────────────── */

const once = rewriteSpotifyEmbeds(
  `https://open.spotify.com/episode/${EPISODE_ID}`,
);
const twice = rewriteSpotifyEmbeds(once);
check("rewrite is idempotent", once, twice);

console.log(`\n${passed} passed, ${failed} failed`);
process.exit(failed > 0 ? 1 : 0);
