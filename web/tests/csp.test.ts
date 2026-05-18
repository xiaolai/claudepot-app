/**
 * CSP allowlist contract — pins the image origins the app actually
 * uses, so a silent drop fails the build instead of breaking prod.
 *
 *   pnpm tsx tests/csp.test.ts
 *
 * Why this file exists
 * --------------------
 * On 2026-05-18 the strict-CSP middleware shipped with an img-src
 * that only allowed Google + GitHub OAuth hosts. Every avatar the
 * app actually renders — user uploads written by setAvatar() and the
 * seeded bot avatars — lives on Vercel Blob, which was not in the
 * allowlist. Result: every avatar on claudepot.com 404'd until a
 * patch landed.
 *
 * Each origin assertion below corresponds to a real writer / source
 * site. Comments name the writer so the next person editing the CSP
 * can trace why the host has to stay.
 *
 * If you add a new image origin (a new CDN, a new OAuth provider,
 * a new markdown source), update the allowlist in src/middleware.ts
 * AND add an assertion here naming the writer.
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

function imgSrc(): string {
  const directive = csp
    .split(";")
    .map((d) => d.trim())
    .find((d) => d.startsWith("img-src "));
  if (!directive) throw new Error("img-src directive missing from CSP");
  return directive;
}

/* ── img-src — every avatar surface must be reachable ────────── */

test("img-src allows 'self' (same-origin app assets)", () => {
  assert.match(imgSrc(), /(?:^|\s)'self'(?:\s|$)/);
});

test("img-src allows data: (inline boring-avatars + sprites)", () => {
  assert.match(imgSrc(), /(?:^|\s)data:(?:\s|$)/);
});

test("img-src allows blob: (in-memory preview before upload)", () => {
  assert.match(imgSrc(), /(?:^|\s)blob:(?:\s|$)/);
});

test("img-src allows Google avatars (OAuth user.image — Google provider)", () => {
  assert.match(imgSrc(), /https:\/\/lh3\.googleusercontent\.com/);
});

test("img-src allows GitHub avatars (OAuth user.image — GitHub provider)", () => {
  assert.match(imgSrc(), /https:\/\/avatars\.githubusercontent\.com/);
});

test("img-src allows raw.githubusercontent.com (markdown image rewrites from GitHub-imported posts)", () => {
  assert.match(imgSrc(), /https:\/\/raw\.githubusercontent\.com/);
});

test("img-src allows Vercel Blob (setAvatar writer + seeded bot avatars — see src/lib/avatars.ts)", () => {
  // Wildcard host because the public store subdomain is project-
  // specific and can rotate. The actual prod host today is
  // iaomvi8nxzu0duzf.public.blob.vercel-storage.com.
  assert.match(imgSrc(), /https:\/\/\*\.public\.blob\.vercel-storage\.com/);
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
