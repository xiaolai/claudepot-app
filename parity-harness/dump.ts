#!/usr/bin/env bun
/**
 * parity-harness/dump.ts — generate a fixture's `expected.json` by
 * running CC's real settings loader against the fixture inputs.
 *
 * STATUS: STUB. The current CC source tree (tested against 2.1.88)
 * cannot be invoked as a library from outside the bun-bundled build:
 *
 *   1. `loadSettingsFromDisk()` is not exported (internal).
 *      `getSettingsWithErrors()` IS exported but calls it through a
 *      session cache that isn't cleared between calls.
 *   2. Source files import `bun:bundle` features (`feature(...)`) that
 *      only resolve through the bundler pipeline. `bun run` on the
 *      raw source fails at import time.
 *   3. The shipped `cli.js` in the published tgz is a minified bundle;
 *      there's no exported API surface.
 *   4. CC has no `--dump-settings` flag (confirmed against 2.1.88 `--help`).
 *   5. Session hooks don't receive effective settings in their env.
 *
 * These together mean a real `dump.ts` requires ONE of:
 *   (a) A CC-side shim installed into $CLAUDE_SRC that re-exports
 *       `loadSettingsFromDisk` + `getMcpConfigsByScope`, then invoked
 *       via `bun --bun $CLAUDE_SRC/parity-shim/dump.ts`.
 *   (b) A CC-side patch that adds a `--dump-effective-settings` flag
 *       and calls through getSettingsWithErrors().
 *   (c) A behavioral probe that drives `claude -p 'noop'` in a
 *       fixture with a distinctive setting value at each scope, plus
 *       a hook that dumps the observed value. Works but is slow and
 *       only reveals a handful of keys per run.
 *
 * Until one of these lands, `expected.json` files are HAND-AUTHORED
 * from a close reading of `utils/settings/settings.ts:645-790`. The
 * Rust `effective_settings::compute` is tested against those goldens
 * by `cargo xtask verify-cc-parity`.
 *
 * When an adapter becomes available, this file becomes:
 *
 *   import { getSettingsWithErrors } from '<adapter-entry>'
 *   const fixture = process.argv[2]
 *   process.env.HOME = join(fixture, 'home')
 *   process.chdir(join(fixture, 'repo'))
 *   process.env.CLAUDE_CODE_SESSION_CACHE = '1' // force cold read
 *   const { settings } = getSettingsWithErrors()
 *   writeFileSync(join(fixture, 'expected.json'), JSON.stringify(settings, null, 2))
 *
 * For now, running this script prints the adapter contract and exits 1.
 */

const fixture = process.argv[2];
if (!fixture) {
  console.error("usage: bun parity-harness/dump.ts <fixture-path>");
  process.exit(2);
}

console.error(`[dump.ts] stub — cannot regenerate ${fixture}/expected.json.`);
console.error("[dump.ts] The CC-side adapter is not installed. See");
console.error("[dump.ts] parity-harness/README.md §4 for the contract.");
console.error("[dump.ts] In the interim, expected.json is hand-authored");
console.error("[dump.ts] against CC source (utils/settings/settings.ts:645-790).");
process.exit(1);
