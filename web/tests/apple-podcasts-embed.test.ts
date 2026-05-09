/**
 * Apple Podcasts embed pre-pass — covers show + episode extraction,
 * the canonical podcasts.apple.com URL shape, and rejection cases.
 *
 * Run with:  pnpm tsx tests/apple-podcasts-embed.test.ts
 */

import assert from "node:assert/strict";
import {
  extractApplePodcastsMatch,
  rewriteApplePodcastsEmbeds,
} from "../src/lib/apple-podcasts-embed";

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

const SHOW_URL =
  "https://podcasts.apple.com/us/podcast/the-tim-ferriss-show/id863897795";
const EPISODE_URL =
  "https://podcasts.apple.com/us/podcast/the-tim-ferriss-show/id863897795?i=1000635467219";

/* ── extractApplePodcastsMatch — accept paths ─────────────────── */

check("show URL", extractApplePodcastsMatch(SHOW_URL), {
  country: "us",
  slug: "the-tim-ferriss-show",
  showId: "863897795",
  episodeId: null,
});

check("episode URL", extractApplePodcastsMatch(EPISODE_URL), {
  country: "us",
  slug: "the-tim-ferriss-show",
  showId: "863897795",
  episodeId: "1000635467219",
});

check(
  "non-US country tolerated",
  extractApplePodcastsMatch(
    "https://podcasts.apple.com/jp/podcast/the-tim-ferriss-show/id863897795",
  ),
  {
    country: "jp",
    slug: "the-tim-ferriss-show",
    showId: "863897795",
    episodeId: null,
  },
);

check(
  "trailing slash tolerated",
  extractApplePodcastsMatch(`${SHOW_URL}/`),
  {
    country: "us",
    slug: "the-tim-ferriss-show",
    showId: "863897795",
    episodeId: null,
  },
);

check(
  "extra query params besides i= are dropped",
  extractApplePodcastsMatch(`${EPISODE_URL}&utm_source=tweet`),
  {
    country: "us",
    slug: "the-tim-ferriss-show",
    showId: "863897795",
    episodeId: "1000635467219",
  },
);

/* ── extractApplePodcastsMatch — reject paths ─────────────────── */

check(
  "non-apple host rejected",
  extractApplePodcastsMatch("https://example.com/us/podcast/foo/id123"),
  null,
);

check(
  "music.apple.com (not podcasts) rejected",
  extractApplePodcastsMatch(
    "https://music.apple.com/us/album/abbey-road/1441164426",
  ),
  null,
);

check(
  "missing id segment rejected",
  extractApplePodcastsMatch(
    "https://podcasts.apple.com/us/podcast/the-tim-ferriss-show",
  ),
  null,
);

check(
  "non-numeric showId rejected",
  extractApplePodcastsMatch(
    "https://podcasts.apple.com/us/podcast/the-tim-ferriss-show/idabc",
  ),
  null,
);

check(
  "non-numeric episode i= dropped (still parses as show)",
  extractApplePodcastsMatch(`${SHOW_URL}?i=abc`),
  {
    country: "us",
    slug: "the-tim-ferriss-show",
    showId: "863897795",
    episodeId: null,
  },
);

check(
  "uppercase country code rejected (canonical lowercase)",
  extractApplePodcastsMatch(
    "https://podcasts.apple.com/US/podcast/the-tim-ferriss-show/id863897795",
  ),
  null,
);

check("garbage URL rejected", extractApplePodcastsMatch("not-a-url"), null);

/* ── rewriteApplePodcastsEmbeds — bare URL paragraph trigger ──── */

const bareShow = rewriteApplePodcastsEmbeds(
  `Subscribe here:\n\n${SHOW_URL}\n\nWeekly drop.`,
);
checkContains(
  "show URL → embed.podcasts.apple.com iframe",
  bareShow,
  `<iframe src="https://embed.podcasts.apple.com/us/podcast/the-tim-ferriss-show/id863897795"`,
);
checkContains(
  "wrapper class set for CSS hookup",
  bareShow,
  `class="proto-applepod-embed"`,
);

const bareEpisode = rewriteApplePodcastsEmbeds(EPISODE_URL);
checkContains(
  "episode URL → embed iframe with ?i=<n>",
  bareEpisode,
  `src="https://embed.podcasts.apple.com/us/podcast/the-tim-ferriss-show/id863897795?i=1000635467219"`,
);
checkContains(
  "episode iframe carries title",
  bareEpisode,
  `title="Apple Podcasts embed"`,
);
checkContains("episode iframe is lazy-loaded", bareEpisode, `loading="lazy"`);
checkContains(
  "episode iframe is sandboxed",
  bareEpisode,
  `sandbox="allow-scripts allow-same-origin allow-popups allow-forms"`,
);

/* ── Reject contexts ──────────────────────────────────────────── */

const insideFence = rewriteApplePodcastsEmbeds(
  `\`\`\`\n${SHOW_URL}\n\`\`\`\n`,
);
checkExcludes("URL inside fenced code block NOT embedded", insideFence, "<iframe");

const indented = rewriteApplePodcastsEmbeds(`    ${SHOW_URL}\n`);
checkExcludes(
  "URL with 4-space indent (code block) NOT embedded",
  indented,
  "<iframe",
);

const inProse = rewriteApplePodcastsEmbeds(
  `Subscribe at ${SHOW_URL} — weekly drops.`,
);
checkExcludes(
  "URL inside prose (not own line) NOT embedded",
  inProse,
  "<iframe",
);

/* ── Idempotency ──────────────────────────────────────────────── */

const once = rewriteApplePodcastsEmbeds(SHOW_URL);
const twice = rewriteApplePodcastsEmbeds(once);
check("rewrite is idempotent", once, twice);

console.log(`\n${passed} passed, ${failed} failed`);
process.exit(failed > 0 ? 1 : 0);
