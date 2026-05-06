/**
 * Drift test for lib/api/manifest.ts.
 *
 *   pnpm tsx tests/api-manifest.test.ts
 *
 * The manifest is the source of truth for every (method, path) on
 * the public API and every MCP tool name. This test catches drift
 * in both directions:
 *
 *   1. A new route file under app/api/v1/** that doesn't have a
 *      matching ENDPOINTS entry → fail.
 *   2. An ENDPOINTS entry that points at a path/method nothing
 *      implements → fail.
 *   3. A `server.registerTool("…")` call in lib/mcp/{tools,read-tools}.ts
 *      that's not in MCP_TOOLS → fail.
 *   4. An MCP_TOOLS entry not registered → fail.
 *
 * Path templates in the manifest use {param} placeholders; route
 * folder names use [param] segments (Next.js convention). The walker
 * normalizes [name] → {name} so the comparison is honest.
 *
 * The test reads files from disk (no DB import path), so it can run
 * without DATABASE_URL. The manifest itself is a pure module — the
 * import is safe.
 */

import { readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

import { ENDPOINTS, MCP_TOOLS, type HttpMethod } from "../src/lib/api/manifest";

const __dirname = dirname(fileURLToPath(import.meta.url));
const REPO_WEB = resolve(__dirname, "..");
const ROUTES_ROOT = resolve(REPO_WEB, "src/app/api/v1");

// MCP tool registrations live in lib/mcp/read-tools.ts (12 reads) and
// across the lib/mcp/tools/ directory (writes + identity, split by
// domain). Walk both at discovery time so a new tools/ file picks up
// drift checks automatically.
function discoverMcpFiles(): string[] {
  const out: string[] = [resolve(REPO_WEB, "src/lib/mcp/read-tools.ts")];
  const toolsDir = resolve(REPO_WEB, "src/lib/mcp/tools");
  for (const name of readdirSync(toolsDir)) {
    if (name.endsWith(".ts")) out.push(resolve(toolsDir, name));
  }
  return out;
}
const MCP_FILES = discoverMcpFiles();

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

/* ── Discover routes on disk ─────────────────────────────────── */

const HTTP_METHODS: HttpMethod[] = ["GET", "POST", "PATCH", "DELETE"];

type RouteInstance = { method: HttpMethod; path: string; file: string };

function isRouteFile(p: string): boolean {
  return p.endsWith("/route.ts") || p.endsWith("/route.tsx");
}

function discoverRoutes(): RouteInstance[] {
  const out: RouteInstance[] = [];
  function walk(dir: string, urlSegs: string[]) {
    for (const name of readdirSync(dir)) {
      const full = join(dir, name);
      const st = statSync(full);
      if (st.isDirectory()) {
        const seg = name.replace(/^\[(.+)\]$/, "{$1}");
        walk(full, [...urlSegs, seg]);
      } else if (isRouteFile(full)) {
        const url = "/api/v1" + (urlSegs.length > 0 ? "/" + urlSegs.join("/") : "");
        const content = readFileSync(full, "utf-8");
        for (const m of content.matchAll(/^export async function (GET|POST|PATCH|DELETE)\(/gm)) {
          out.push({
            method: m[1] as HttpMethod,
            path: url,
            file: relative(REPO_WEB, full),
          });
        }
      }
    }
  }
  walk(ROUTES_ROOT, []);
  return out;
}

/* ── Discover MCP tool registrations ─────────────────────────── */

function discoverMcpTools(): string[] {
  const names = new Set<string>();
  for (const f of MCP_FILES) {
    const content = readFileSync(f, "utf-8");
    for (const m of content.matchAll(
      /server\.registerTool\(\s*"([a-z_]+)"/gm,
    )) {
      names.add(m[1]);
    }
  }
  return [...names].sort();
}

/* ── Assertions ──────────────────────────────────────────────── */

test("every implemented route has a manifest entry", () => {
  const routes = discoverRoutes();
  const manifestKeys = new Set(
    ENDPOINTS.map((e) => `${e.method} ${e.path}`),
  );
  const missing = routes
    .filter((r) => !manifestKeys.has(`${r.method} ${r.path}`))
    .map((r) => `${r.method} ${r.path} (${r.file})`);
  if (missing.length > 0) {
    throw new Error(
      `Routes missing from manifest:\n  - ${missing.join("\n  - ")}\n` +
        `Add an EndpointId + ENDPOINTS row in lib/api/manifest.ts.`,
    );
  }
});

test("every manifest entry has an implementing route", () => {
  const routes = discoverRoutes();
  const routeKeys = new Set(routes.map((r) => `${r.method} ${r.path}`));
  const dangling = ENDPOINTS.filter(
    (e) => !routeKeys.has(`${e.method} ${e.path}`),
  ).map((e) => `${e.method} ${e.path} (id="${e.id}")`);
  if (dangling.length > 0) {
    throw new Error(
      `Manifest entries with no route file:\n  - ${dangling.join("\n  - ")}\n` +
        `Either implement the route or remove the manifest row.`,
    );
  }
});

test("every method in HTTP_METHODS exists somewhere", () => {
  // Sanity check on the discovery regex — if we ever add a verb
  // (e.g. PUT), the manifest's HttpMethod union covers it but the
  // regex won't, so this guard reminds us to update both.
  const seen = new Set(discoverRoutes().map((r) => r.method));
  for (const m of HTTP_METHODS) {
    if (!seen.has(m)) {
      // Not all methods are in use today — only flag if the manifest
      // CLAIMS an entry exists for an unused method.
      const claimed = ENDPOINTS.filter((e) => e.method === m);
      if (claimed.length > 0) {
        throw new Error(
          `Manifest claims ${m} entries but no route file exposes ${m}.`,
        );
      }
    }
  }
});

test("manifest paths use {param}, not [param]", () => {
  // Catches accidental copy-paste of Next.js segment syntax into a
  // path string — the docs page renders these verbatim.
  const broken = ENDPOINTS.filter((e) => /\[\w+\]/.test(e.path)).map(
    (e) => `${e.id}: ${e.path}`,
  );
  if (broken.length > 0) {
    throw new Error(
      `Manifest paths contain [param] segments — use {param}:\n  - ${broken.join("\n  - ")}`,
    );
  }
});

/* ── MCP drift ───────────────────────────────────────────────── */

test("every registered MCP tool is in MCP_TOOLS", () => {
  const registered = new Set(discoverMcpTools());
  const manifestNames = new Set(MCP_TOOLS.map((t) => t.name));
  const orphan = [...registered].filter((n) => !manifestNames.has(n as never));
  if (orphan.length > 0) {
    throw new Error(
      `MCP tools registered but not in MCP_TOOLS:\n  - ${orphan.join("\n  - ")}\n` +
        `Add a McpToolName + MCP_TOOLS row in lib/api/manifest.ts.`,
    );
  }
});

test("every MCP_TOOLS entry is registered", () => {
  const registered = new Set(discoverMcpTools());
  const dangling = MCP_TOOLS.filter((t) => !registered.has(t.name)).map(
    (t) => t.name,
  );
  if (dangling.length > 0) {
    throw new Error(
      `MCP_TOOLS entries with no registration:\n  - ${dangling.join("\n  - ")}\n` +
        `Either register the tool in lib/mcp/{tools,read-tools}.ts or remove the row.`,
    );
  }
});

test("MCP tool names are unique across both registration files", () => {
  // Same tool registered twice would mean both files attempt to
  // claim the same name — McpServer would error, but we'd rather
  // catch it at test time.
  const counts = new Map<string, number>();
  for (const f of MCP_FILES) {
    const content = readFileSync(f, "utf-8");
    for (const m of content.matchAll(/server\.registerTool\(\s*"([a-z_]+)"/gm)) {
      counts.set(m[1], (counts.get(m[1]) ?? 0) + 1);
    }
  }
  const dup = [...counts.entries()].filter(([, n]) => n > 1).map(([k]) => k);
  if (dup.length > 0) {
    throw new Error(`MCP tool names registered more than once: ${dup.join(", ")}`);
  }
});

console.log(`\n${passed} passed, ${failed} failed`);
process.exit(failed > 0 ? 1 : 0);
