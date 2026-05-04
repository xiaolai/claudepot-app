// Empty export marks this file as a module so top-level await is allowed.
export {};

/**
 * MCP client interop test against the deployed MCP server.
 *
 *   pnpm exec tsx --env-file=.env.local scripts/smoke-mcp-client.ts [BASE_URL]
 *
 * Defaults to https://claudepot.com. Uses the OFFICIAL @modelcontextprotocol/sdk
 * client + StreamableHTTPClientTransport — same code path Claude Desktop,
 * Cursor, Windsurf, etc. take when they connect to a remote MCP server.
 *
 * If this passes, the server speaks proper MCP and any spec-compliant
 * client should be able to talk to it.
 */

import { Client } from "@modelcontextprotocol/sdk/client/index.js";
import { StreamableHTTPClientTransport } from "@modelcontextprotocol/sdk/client/streamableHttp.js";

const BASE_URL = process.argv[2] ?? "https://claudepot.com";
const TOKEN = process.env.SHANNON_API_TOKEN;

if (!TOKEN) {
  console.error("✗ SHANNON_API_TOKEN missing from environment.");
  process.exit(2);
}

console.log(`> MCP client test against ${BASE_URL}/api/mcp`);

const transport = new StreamableHTTPClientTransport(
  new URL(`${BASE_URL}/api/mcp`),
  {
    requestInit: {
      headers: { Authorization: `Bearer ${TOKEN}` },
    },
  },
);

const client = new Client({
  name: "smoke-mcp-client",
  version: "0.1.0",
});

let allPassed = true;
const fail = (msg: string) => {
  console.error(`✗ ${msg}`);
  allPassed = false;
};

try {
  await client.connect(transport);
  console.log(`✓ Connected. Server: ${JSON.stringify(client.getServerVersion())}`);

  /* ── tools/list ────────────────────────────────────────────── */
  const { tools } = await client.listTools();
  const names = tools.map((t) => t.name).sort();
  console.log(`✓ tools/list returned: ${names.join(", ")}`);
  if (!names.includes("submit_link") || !names.includes("me")) {
    fail(`Expected at least submit_link + me, got: ${names.join(", ")}`);
  }

  /* ── me ──────────────────────────────────────────────────── */
  const meRes = await client.callTool({ name: "me", arguments: {} });
  const meText = (meRes.content as Array<{ type: string; text?: string }>)[0]?.text;
  if (!meText) {
    fail(`me: no text content in response`);
  } else {
    const meJson = JSON.parse(meText);
    console.log(`✓ me returned @${meJson.username} (role: ${meJson.role})`);
    if (!meJson.username) fail("me: missing username");
    if (!Array.isArray(meJson.scopes)) fail("me: missing scopes");
  }

  /* ── unknown tool: either thrown or isError is acceptable ── */
  let unknownRejected = false;
  let unknownDetail = "";
  try {
    const res = await client.callTool({
      name: "definitely_not_a_real_tool",
      arguments: {},
    });
    if (res.isError) {
      unknownRejected = true;
      unknownDetail = "isError flag set";
    }
  } catch (err) {
    unknownRejected = true;
    unknownDetail =
      err instanceof Error ? err.message.slice(0, 80) : String(err);
  }
  if (unknownRejected) {
    console.log(`✓ Unknown tool rejected (${unknownDetail})`);
  } else {
    fail("Unknown tool was accepted — server is too permissive");
  }
} catch (err) {
  console.error(`✗ MCP client test crashed:`, err);
  process.exit(1);
} finally {
  await client.close().catch(() => {});
}

if (!allPassed) {
  console.error(`\n✗ MCP client test failed`);
  process.exit(1);
}
console.log(`\n✓ all checks passed — server is interop-compatible`);
