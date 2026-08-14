#!/usr/bin/env node
/**
 * Discover and run every `tests/*.test.ts`.
 *
 * # Why discovery instead of a list
 *
 * `package.json`'s `test` script used to be 23 filenames chained with
 * `&&`. Adding a test file did not add it to the suite — you had to
 * remember the second edit, and three people did not:
 *
 *   - `tests/username.test.ts` (24 assertions: reserved names,
 *     self-rename cooldown — the impersonation surface of a public
 *     site)
 *   - `tests/editorial-routing.test.ts` (12 assertions: moderation
 *     gate thresholds)
 *   - `tests/social-format.test.ts` (9 assertions: X / Bluesky length
 *     budgets)
 *
 * All three passed when finally run, so nothing was ever red. That is
 * the bad case, not the good one: 45 assertions sat in the repo
 * looking like coverage while `pnpm test` reported green without them,
 * and CI runs only `pnpm test`.
 *
 * A list of files is a cache of the directory. This reads the
 * directory.
 *
 * # Scope
 *
 * Top-level `tests/*.test.ts` only. `tests/integration/` stays out
 * deliberately — those need `--env-file=.env.local` and a live Neon
 * connection, so they cannot run in CI and have their own
 * `test:integration` script.
 */
import { readdirSync } from "node:fs";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, join, resolve } from "node:path";

const webRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const testsDir = join(webRoot, "tests");

const files = readdirSync(testsDir, { withFileTypes: true })
  .filter((e) => e.isFile() && e.name.endsWith(".test.ts"))
  .map((e) => e.name)
  .sort();

if (files.length === 0) {
  // A discovery run that finds nothing must fail loudly. Exiting 0
  // here would reproduce the exact failure this script replaces: a
  // green suite that ran no tests.
  console.error("run-tests: no tests/*.test.ts found — discovery is broken");
  process.exit(1);
}

console.log(`run-tests: ${files.length} test file(s)\n`);

const failed = [];
for (const name of files) {
  const r = spawnSync("tsx", [join("tests", name)], {
    cwd: webRoot,
    stdio: "inherit",
    env: process.env,
    shell: process.platform === "win32",
  });
  if (r.status !== 0) failed.push(name);
}

if (failed.length > 0) {
  console.error(`\nrun-tests: ${failed.length} file(s) failed:`);
  for (const f of failed) console.error(`  tests/${f}`);
  process.exit(1);
}

console.log(`\nrun-tests: all ${files.length} file(s) passed`);
