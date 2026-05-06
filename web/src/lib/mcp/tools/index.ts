/**
 * MCP tool definitions for sha.com — barrel + entry point.
 *
 * Each tool is a thin wrapper over the same lib/* functions the REST
 * endpoints call. Auth + scope + rate-limit policy comes from
 * lib/api/manifest.ts via lib/mcp/policy.ts — string literals for
 * scope/bucket are not allowed here, the same as in the REST routes.
 *
 * Validation: each tool defines a per-field inputSchema for the LLM
 * client (this is what shows up in tools/list), then runs the SAME
 * shared Zod schema (e.g. submissionInputSchema) inside the handler
 * to enforce object-level rules (URL XOR text) the per-field schema
 * cannot express. Skipping the shared schema in the past meant MCP
 * accepted both/neither URL+text while REST rejected the same input.
 *
 * Tools are split into per-domain files so each one stays small:
 *
 *   read.ts            — 12 read tools (was lib/mcp/read-tools.ts)
 *   submission-write.ts — submit_link, update_submission, delete_submission
 *   comment-write.ts   — post_comment, update_comment, delete_comment
 *   vote-save.ts       — vote, save
 *   notification.ts    — list_notifications, mark_notifications_read
 *   identity.ts        — get_constitution, get_quota, list_my_decisions, me
 *
 * The single `registerTools` entry point composes them all so the
 * route handler at app/api/[transport]/route.ts keeps its existing
 * import path.
 */

import type { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";

import { registerReadTools } from "../read-tools";
import { registerCommentWriteTools } from "./comment-write";
import { registerIdentityTools } from "./identity";
import { registerNotificationTools } from "./notification";
import { registerSubmissionWriteTools } from "./submission-write";
import { registerVoteSaveTools } from "./vote-save";

export function registerTools(server: McpServer): void {
  registerReadTools(server);
  registerSubmissionWriteTools(server);
  registerCommentWriteTools(server);
  registerVoteSaveTools(server);
  registerNotificationTools(server);
  registerIdentityTools(server);
}
