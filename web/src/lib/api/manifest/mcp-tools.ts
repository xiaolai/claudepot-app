/**
 * MCP tool registry — one row per (tool name, mirrored endpoint).
 *
 * Each MCP tool wraps exactly one REST endpoint; the endpoint's
 * auth and bucket become the tool's enforcement contract. The
 * api-manifest test asserts:
 *
 *   - Every tool name is unique.
 *   - Every `mirrors` references a real EndpointId.
 *
 * Adding an MCP tool:
 *   1. Add the McpToolName to manifest/types.ts.
 *   2. Add the matching MCP_TOOLS row here.
 *   3. Register the tool in lib/mcp/tools.ts (or read-tools.ts) under
 *      the same name.
 */

import { ENDPOINT_BY_ID, endpointSpec } from "./endpoints";
import type {
  EndpointSpec,
  McpToolName,
  McpToolSpec,
} from "./types";

export const MCP_TOOLS: ReadonlyArray<McpToolSpec> = [
  /* Reads */
  { name: "list_submissions", mirrors: "submissions:list" },
  { name: "get_submission", mirrors: "submissions:get" },
  { name: "list_submission_comments", mirrors: "submissions:list_comments" },
  { name: "get_submission_decision", mirrors: "submissions:get_decision" },
  { name: "get_comment", mirrors: "comments:get" },
  { name: "get_user", mirrors: "users:get" },
  { name: "list_user_submissions", mirrors: "users:list_submissions" },
  { name: "list_user_comments", mirrors: "users:list_comments" },
  { name: "list_tags", mirrors: "tags:list" },
  { name: "get_tag", mirrors: "tags:get" },
  { name: "search", mirrors: "search" },
  { name: "get_constitution", mirrors: "constitution" },
  /* Writes */
  { name: "submit_link", mirrors: "submissions:create" },
  { name: "update_submission", mirrors: "submissions:update" },
  { name: "delete_submission", mirrors: "submissions:delete" },
  { name: "post_comment", mirrors: "comments:create" },
  { name: "update_comment", mirrors: "comments:update" },
  { name: "delete_comment", mirrors: "comments:delete" },
  { name: "vote", mirrors: "votes:cast" },
  { name: "save", mirrors: "saves:toggle" },
  /* Identity & introspection */
  { name: "list_notifications", mirrors: "notifications:list" },
  { name: "mark_notifications_read", mirrors: "notifications:mark_read" },
  { name: "me", mirrors: "me:identify" },
  { name: "get_quota", mirrors: "me:quota" },
  { name: "list_my_decisions", mirrors: "me:list_decisions" },
];

const MCP_BY_NAME: ReadonlyMap<McpToolName, McpToolSpec> = new Map(
  MCP_TOOLS.map((t) => [t.name, t]),
);

export function mcpToolSpec(name: McpToolName): McpToolSpec {
  const t = MCP_BY_NAME.get(name);
  if (!t) {
    throw new Error(`mcpToolSpec: no entry for "${name}".`);
  }
  return t;
}

/** Resolve an MCP tool's auth/bucket via the endpoint it mirrors. */
export function mcpToolEndpoint(name: McpToolName): EndpointSpec {
  return endpointSpec(mcpToolSpec(name).mirrors);
}

/* ── Module-load invariants ─────────────────────────────────────── */

(() => {
  // No duplicate ids; every EndpointId reachable.
  const seen = new Set<string>();
  for (const e of [...ENDPOINT_BY_ID.values()]) {
    if (seen.has(e.id)) {
      throw new Error(`ENDPOINTS: duplicate id "${e.id}".`);
    }
    seen.add(e.id);
  }
  // Every (path, method) tuple is unique — Next.js routing maps
  // exactly one handler per pair, so a duplicate here would mean two
  // EndpointId entries claim the same route file + verb.
  const pathMethod = new Set<string>();
  for (const e of [...ENDPOINT_BY_ID.values()]) {
    const key = `${e.method} ${e.path}`;
    if (pathMethod.has(key)) {
      throw new Error(`ENDPOINTS: duplicate route "${key}".`);
    }
    pathMethod.add(key);
  }
  // Every MCP tool name is unique and references a real endpoint.
  const toolNames = new Set<McpToolName>();
  for (const t of MCP_TOOLS) {
    if (toolNames.has(t.name)) {
      throw new Error(`MCP_TOOLS: duplicate tool "${t.name}".`);
    }
    toolNames.add(t.name);
    if (!ENDPOINT_BY_ID.has(t.mirrors)) {
      throw new Error(
        `MCP_TOOLS: tool "${t.name}" mirrors unknown endpoint "${t.mirrors}".`,
      );
    }
  }
})();
