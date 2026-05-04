// Empty export marks this file as a module so top-level await is allowed.
export {};

/**
 * Verify a SHANNON_API_TOKEN against the public API.
 *
 *   pnpm exec tsx --env-file=.env.local scripts/api-ping.ts [BASE_URL]
 *
 * Defaults to https://claudepot.com. Override with the first arg, e.g.
 *   pnpm exec tsx --env-file=.env.local scripts/api-ping.ts http://localhost:3000
 *
 * Reads SHANNON_API_TOKEN from process.env, calls GET /api/v1/me, and
 * prints the resolved identity + token metadata. Exits non-zero on any
 * failure so it's safe to use in scripts / CI as a credential check.
 */

const BASE_URL = process.argv[2] ?? "https://claudepot.com";
const TOKEN = process.env.SHANNON_API_TOKEN;

if (!TOKEN) {
  console.error("✗ SHANNON_API_TOKEN missing from environment");
  console.error("  Add it to .env.local or pass via env, then re-run.");
  process.exit(2);
}

const t0 = Date.now();
const res = await fetch(`${BASE_URL}/api/v1/me`, {
  headers: { Authorization: `Bearer ${TOKEN}` },
});
const elapsed = Date.now() - t0;

const body = await res.text();
let parsed: unknown;
try {
  parsed = JSON.parse(body);
} catch {
  console.error(`✗ Non-JSON response (${res.status} in ${elapsed}ms):`);
  console.error(body.slice(0, 500));
  process.exit(1);
}

if (!res.ok) {
  console.error(`✗ HTTP ${res.status} in ${elapsed}ms`);
  console.error(JSON.stringify(parsed, null, 2));
  process.exit(1);
}

console.log(`✓ ${BASE_URL}/api/v1/me  (${res.status} in ${elapsed}ms)`);
console.log(JSON.stringify(parsed, null, 2));
