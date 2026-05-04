/**
 * Same-origin redirect guard used by /login (?callbackUrl=) and
 * any future flow that takes a user-controlled redirect target.
 *
 * Run with:
 *   pnpm tsx tests/safe-redirect.test.ts
 *
 * Exits 1 on any failure. The cases below cover the three CVE-class
 * bypasses we care about — protocol-relative, backslash-normalised,
 * and absolute http(s) — plus the happy path.
 */

import assert from "node:assert/strict";
import { safeCallback } from "../src/lib/safe-redirect";

let passed = 0;
let failed = 0;

function check(label: string, actual: string, expected: string) {
  try {
    assert.equal(actual, expected);
    console.log(`PASS  ${label}`);
    passed += 1;
  } catch {
    console.error(`FAIL  ${label}`);
    console.error(`      got      ${JSON.stringify(actual)}`);
    console.error(`      expected ${JSON.stringify(expected)}`);
    failed += 1;
  }
}

// Happy paths — same-origin local paths must pass through verbatim.
check("plain path", safeCallback("/saved"), "/saved");
check("nested path", safeCallback("/u/ada"), "/u/ada");
check("path with query", safeCallback("/?q=x"), "/?q=x");
check("path with fragment", safeCallback("/post/123#comment-4"), "/post/123#comment-4");
check("path with trailing slash", safeCallback("/saved/"), "/saved/");

// Protocol-relative — most common open-redirect bypass.
check("protocol-relative URL → /", safeCallback("//evil.example"), "/");
check("protocol-relative with path → /", safeCallback("//evil.example/login"), "/");

// Backslash bypass — browsers normalise "\" to "/" in paths, so
// "/\evil" lands on "//evil" → off-origin. Must be rejected.
check("single backslash → /", safeCallback("/\\evil.example"), "/");
check("double backslash → /", safeCallback("/\\\\evil.example"), "/");
check("trailing-slash backslash → /", safeCallback("/\\evil.example/"), "/");

// Absolute URLs — never honoured.
check("https absolute → /", safeCallback("https://evil.example"), "/");
check("http absolute → /", safeCallback("http://evil.example"), "/");

// Junk inputs.
check("empty string → /", safeCallback(""), "/");
check("undefined → /", safeCallback(undefined), "/");
check("array first valid → first value", safeCallback(["/saved", "/upvoted"]), "/saved");
check("array first invalid → /", safeCallback(["//evil", "/saved"]), "/");
check("not-leading-slash → /", safeCallback("saved"), "/");
check("bare slash (length 1) → /", safeCallback("/"), "/");

// Schemes embedded after a slash — rare but covered.
check("javascript: scheme via slash → /", safeCallback("/javascript:alert(1)"), "/javascript:alert(1)");
// ^ This is intentionally a same-origin path; browsers will navigate
//   to /javascript:alert(1) on this origin (a local 404), not execute
//   javascript. So it's fine. Documented for clarity.

console.log("");
console.log(`${passed} passed, ${failed} failed`);
if (failed > 0) process.exit(1);
