#!/usr/bin/env bun
/**
 * parity-harness/dump.ts — drive the installed Claude Code with a fixture's
 * settings layers and read back what CC itself merged.
 *
 * The adapter gap §4 of the README waited on is open: since at least 2.1.272
 * CC answers the SDK control request `{ subtype: "get_settings" }` with
 * `effective` (its disk merge) and `sources` (the raw per-source layers, low
 * to high). This needs no credentials and makes no model call — the session
 * receives two control requests and no user message, then stdin closes.
 *
 * What it can drive, and what it cannot:
 *
 * | layer         | how                                                    |
 * |---------------|--------------------------------------------------------|
 * | user          | `$CLAUDE_CONFIG_DIR/settings.json` in a sandbox        |
 * | project/local | `<sandbox project>/.claude/settings{,.local}.json`     |
 * | flag          | `--settings <json>`                                    |
 * | plugin_base   | a sandbox plugin passed with `--plugin-dir`            |
 * | policy        | NOT drivable: the managed file lives at a fixed system |
 * |               | path (the override hook is compiled out), and remote,  |
 * |               | MDM and HKCU need an org, a profile, or Windows        |
 *
 * A fixture with any non-empty policy entry is reported as not drivable
 * rather than run with that layer silently missing. A run whose `sources`
 * contain anything the fixture did not provide — a managed file or MDM
 * profile on the machine — fails: the comparison would be meaningless.
 *
 * Usage:
 *   bun parity-harness/dump.ts <fixture-dir>             # print CC's merge
 *   bun parity-harness/dump.ts --check [<fixture-dir>…]  # compare to expected.json
 *
 * `--check` with no directories checks every fixture and exits non-zero on
 * any mismatch; not-drivable fixtures are listed, never counted as passing.
 */
import { spawn } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, join, resolve } from "node:path";

type Json = null | boolean | number | string | Json[] | { [k: string]: Json };
interface Input {
  plugin_base: Json;
  user: Json;
  project: Json;
  local: Json;
  flag: Json;
  policy: { origin: string; value: Json }[];
}
type Outcome =
  | { kind: "merged"; effective: Json; sources: string[]; errors: Json[] }
  | { kind: "not_drivable"; reason: string };

const HERE = import.meta.dir;
const CLAUDE = process.env.CLAUDE_BIN ?? "claude";

function writeJson(path: string, value: Json) {
  mkdirSync(join(path, ".."), { recursive: true });
  writeFileSync(path, JSON.stringify(value, null, 2));
}

async function mergeWithClaudeCode(input: Input): Promise<Outcome> {
  const policy = input.policy.filter((p) => p.value !== null);
  if (policy.length > 0) {
    return {
      kind: "not_drivable",
      reason: `policy layer(s) ${policy.map((p) => p.origin).join(", ")} cannot be installed without root, an org, an MDM profile or Windows`,
    };
  }
  const box = mkdtempSync(join(tmpdir(), "cc-parity-"));
  try {
    const cfg = join(box, "cfg");
    const proj = join(box, "proj");
    mkdirSync(cfg, { recursive: true });
    mkdirSync(join(proj, ".claude"), { recursive: true });
    const projReal = realpathSync(proj);
    // Trust the sandbox project so nothing is dropped for being untrusted.
    writeJson(join(cfg, ".claude.json"), {
      projects: { [projReal]: { hasTrustDialogAccepted: true } },
    });
    if (input.user !== null) writeJson(join(cfg, "settings.json"), input.user);
    if (input.project !== null) writeJson(join(proj, ".claude", "settings.json"), input.project);
    if (input.local !== null) writeJson(join(proj, ".claude", "settings.local.json"), input.local);

    const args = ["-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose"];
    if (input.flag !== null) args.push("--settings", JSON.stringify(input.flag));
    if (input.plugin_base !== null) {
      const plugin = join(box, "plugin");
      writeJson(join(plugin, ".claude-plugin", "plugin.json"), {
        name: "parity-base",
        version: "0.0.0",
        description: "parity harness plugin settings base",
      });
      writeJson(join(plugin, "settings.json"), input.plugin_base);
      args.push("--plugin-dir", plugin);
    }

    const env: Record<string, string> = {};
    for (const [k, v] of Object.entries(process.env)) {
      // Nothing from the caller's own Claude Code may leak into the merge.
      if (v === undefined || k.startsWith("CLAUDE_") || k.startsWith("ANTHROPIC_")) continue;
      env[k] = v;
    }
    env.CLAUDE_CONFIG_DIR = cfg;

    const child = spawn(CLAUDE, args, { cwd: projReal, env, stdio: ["pipe", "pipe", "pipe"] });
    let out = "";
    let err = "";
    child.stdout.on("data", (b) => (out += b));
    child.stderr.on("data", (b) => (err += b));
    const timer = setTimeout(() => child.kill("SIGKILL"), 60_000);
    child.stdin.write(`${JSON.stringify({ type: "control_request", request_id: "init", request: { subtype: "initialize" } })}\n`);
    child.stdin.write(`${JSON.stringify({ type: "control_request", request_id: "settings", request: { subtype: "get_settings" } })}\n`);
    child.stdin.end();
    const code: number = await new Promise((r) => child.on("close", (c) => r(c ?? -1)));
    clearTimeout(timer);

    for (const line of out.split("\n")) {
      if (!line.trim()) continue;
      let msg: any;
      try {
        msg = JSON.parse(line);
      } catch {
        continue;
      }
      const res = msg?.response;
      if (msg?.type !== "control_response" || res?.request_id !== "settings") continue;
      if (res.subtype !== "success") {
        throw new Error(`get_settings failed: ${JSON.stringify(res).slice(0, 300)}`);
      }
      const body = res.response;
      const sources: string[] = (body.sources ?? []).map((s: any) => s.source);
      return { kind: "merged", effective: body.effective ?? {}, sources, errors: body.errors ?? [] };
    }
    throw new Error(`no get_settings response (exit ${code}): ${err.trim().slice(0, 400)}`);
  } finally {
    rmSync(box, { recursive: true, force: true });
  }
}

const EXPECTED_SOURCES: Record<keyof Input, string | null> = {
  user: "userSettings",
  project: "projectSettings",
  local: "localSettings",
  flag: "flagSettings",
  plugin_base: null, // plugin settings are merged before the source list
  policy: "policySettings",
};

function canonical(v: Json): string {
  if (Array.isArray(v)) return `[${v.map(canonical).join(",")}]`;
  if (v && typeof v === "object") {
    return `{${Object.keys(v).sort().map((k) => `${JSON.stringify(k)}:${canonical((v as any)[k])}`).join(",")}}`;
  }
  return JSON.stringify(v);
}

async function check(dir: string): Promise<"pass" | "fail" | "not_drivable"> {
  const input: Input = JSON.parse(readFileSync(join(dir, "input.json"), "utf8"));
  const expected: Json = JSON.parse(readFileSync(join(dir, "expected.json"), "utf8"));
  const name = basename(dir);
  const outcome = await mergeWithClaudeCode(input);
  if (outcome.kind === "not_drivable") {
    console.log(`  -     ${name}: not drivable — ${outcome.reason}`);
    return "not_drivable";
  }
  const allowed = new Set(
    (Object.keys(EXPECTED_SOURCES) as (keyof Input)[])
      .filter((k) => k !== "policy" && input[k] !== null && EXPECTED_SOURCES[k])
      .map((k) => EXPECTED_SOURCES[k] as string),
  );
  // A fixture CC rejects on validation is testing the schema, not the
  // merge: the rejected value never reaches `effective`. Report it as its
  // own failure so it is never mistaken for a precedence finding.
  if (outcome.errors.length > 0) {
    console.log(`  FAIL  ${name}: Claude Code rejected the fixture's settings — fix the inputs, not the merge:`);
    for (const e of outcome.errors) console.log(`        ${JSON.stringify(e).slice(0, 240)}`);
    return "fail";
  }
  const stray = outcome.sources.filter((s) => !allowed.has(s));
  if (stray.length > 0) {
    console.log(`  FAIL  ${name}: Claude Code merged sources the fixture did not provide: ${stray.join(", ")}`);
    return "fail";
  }
  if (canonical(outcome.effective) !== canonical(expected)) {
    console.log(`  FAIL  ${name}: Claude Code merged\n        ${canonical(outcome.effective)}\n      expected\n        ${canonical(expected)}`);
    return "fail";
  }
  console.log(`  ok    ${name}`);
  return "pass";
}

const argv = process.argv.slice(2);
if (argv[0] === "--check") {
  const fixtures = argv.length > 1
    ? argv.slice(1).map((d) => resolve(d))
    : readdirSync(join(HERE, "fixtures")).sort().map((d) => join(HERE, "fixtures", d));
  const results = { pass: 0, fail: 0, not_drivable: 0 };
  for (const dir of fixtures) {
    if (!existsSync(join(dir, "input.json"))) continue;
    results[await check(dir)]++;
  }
  console.log(`dump --check: ${results.pass} match, ${results.fail} differ, ${results.not_drivable} not drivable`);
  if (results.pass === 0) {
    console.log("dump --check: nothing was compared — refusing to report success");
    process.exit(1);
  }
  process.exit(results.fail > 0 ? 1 : 0);
} else if (argv.length === 1) {
  const input: Input = JSON.parse(readFileSync(join(resolve(argv[0]), "input.json"), "utf8"));
  console.log(JSON.stringify(await mergeWithClaudeCode(input), null, 2));
} else {
  console.error("usage: bun parity-harness/dump.ts <fixture-dir> | --check [<fixture-dir>…]");
  process.exit(2);
}
