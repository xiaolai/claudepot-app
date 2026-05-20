/**
 * CSP contract — pins the load-bearing shape of the header so an
 * accidental loosening of script-src or an accidental re-tightening
 * of img-src/media-src fails the build instead of breaking prod.
 *
 *   pnpm tsx tests/csp.test.ts
 *
 * Why this file exists
 * --------------------
 * On 2026-05-18 the strict-CSP middleware shipped with an img-src
 * that only allowed Google + GitHub OAuth hosts. Every avatar the
 * app actually renders — user uploads written by setAvatar() and the
 * seeded bot avatars on Vercel Blob — 404'd until a patch landed.
 *
 * On 2026-05-20, img-src was relaxed to `https:` because claudepot
 * is a content aggregator and an enumerated allowlist of publisher
 * CDNs is structurally unmaintainable. media-src followed for the
 * same reason. The hardening floor (script-src nonce + strict-
 * dynamic, default-src/object-src/frame-ancestors/base-uri/
 * form-action) is unchanged — those are what actually block XSS.
 */

import assert from "node:assert/strict";

import { buildCsp } from "../src/middleware";

let passed = 0;
let failed = 0;
function test(name: string, fn: () => void) {
  try {
    fn();
    console.log(`PASS  ${name}`);
    passed += 1;
  } catch (err) {
    console.error(`FAIL  ${name}`);
    console.error(`      ${err instanceof Error ? err.message : String(err)}`);
    failed += 1;
  }
}

const NONCE = "test-nonce-aaaa==";
const csp = buildCsp(NONCE);

function directive(name: string): string {
  const found = csp
    .split(";")
    .map((d) => d.trim())
    .find((d) => d.startsWith(`${name} `));
  if (!found) throw new Error(`${name} directive missing from CSP`);
  return found;
}

/* ── img-src — open https:, aggregator surface ───────────────── */

test("img-src allows 'self' (same-origin app assets)", () => {
  assert.match(directive("img-src"), /(?:^|\s)'self'(?:\s|$)/);
});

test("img-src allows data: (inline boring-avatars + sprites)", () => {
  assert.match(directive("img-src"), /(?:^|\s)data:(?:\s|$)/);
});

test("img-src allows blob: (in-memory preview before upload)", () => {
  assert.match(directive("img-src"), /(?:^|\s)blob:(?:\s|$)/);
});

test("img-src allows any HTTPS origin (aggregator — arbitrary publisher CDNs)", () => {
  assert.match(directive("img-src"), /(?:^|\s)https:(?:\s|$)/);
});

/* ── media-src — same shape as img-src, for <audio>/<video> ──── */

test("media-src allows 'self' (same-origin media)", () => {
  assert.match(directive("media-src"), /(?:^|\s)'self'(?:\s|$)/);
});

test("media-src allows any HTTPS origin (aggregator — arbitrary media CDNs)", () => {
  assert.match(directive("media-src"), /(?:^|\s)https:(?:\s|$)/);
});

/* ── script-src — hydration must survive the nonce flow ──────── */

test("script-src interpolates the per-request nonce", () => {
  assert.ok(
    csp.includes(`'nonce-${NONCE}'`),
    "expected script-src to carry the runtime-generated nonce",
  );
});

test("script-src keeps 'strict-dynamic' (so nonced scripts can load /_next chunks)", () => {
  assert.match(csp, /script-src[^;]*'strict-dynamic'/);
});

/* ── Hardening invariants — locked down even if someone edits ─── */

test("default-src is 'self' (no third-party fallback)", () => {
  assert.match(csp, /(?:^|;\s*)default-src 'self'(?:\s|;|$)/);
});

test("object-src is 'none' (no plugin content)", () => {
  assert.match(csp, /object-src 'none'/);
});

test("frame-ancestors is 'none' (clickjacking floor)", () => {
  assert.match(csp, /frame-ancestors 'none'/);
});

test("base-uri is 'self' (defeats <base> injection)", () => {
  assert.match(csp, /base-uri 'self'/);
});

test("form-action is 'self' (no off-site POSTs)", () => {
  assert.match(csp, /form-action 'self'/);
});

console.log(`\n${passed} passed, ${failed} failed`);
process.exit(failed > 0 ? 1 : 0);
