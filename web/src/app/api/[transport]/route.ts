/**
 * MCP server endpoint — /api/mcp (streamable HTTP).
 *
 * The dynamic [transport] segment catches /api/mcp and (optionally) other
 * MCP transport paths derived from basePath. Static neighbors (/api/v1/*,
 * /api/auth/*, /api/cron/*, /api/og/*, /api/rss/*) take precedence in the
 * Next.js router, so this dynamic catch-all only fires on paths the
 * adapter actually claims.
 *
 * SSE is disabled — it's deprecated by the MCP spec (2025-03-26) and
 * adds Redis as a dependency for resumability we don't need.
 *
 * Auth: Bearer token via withMcpAuth → verifyClaudepotToken adapts our
 * api_tokens row to MCP's AuthInfo. Tool handlers read user / token /
 * scope from extra.authInfo.
 */

import { createMcpHandler, withMcpAuth } from "mcp-handler";

import { registerTools } from "@/lib/mcp/tools";
import { verifyClaudepotToken } from "@/lib/mcp/auth";

const baseHandler = createMcpHandler(
  (server) => {
    registerTools(server);
  },
  {
    serverInfo: {
      name: "sha.com",
      version: "0.1.0",
    },
  },
  {
    basePath: "/api",
    disableSse: true,
    verboseLogs: process.env.NODE_ENV !== "production",
  },
);

const handler = withMcpAuth(baseHandler, verifyClaudepotToken, {
  required: true,
});

export { handler as GET, handler as POST, handler as DELETE };
