/**
 * MCP tool definitions for sha.com.
 *
 * Each tool is a thin wrapper over the same lib/* functions the REST
 * endpoints call. The MCP and REST surfaces stay in 1:1 lockstep by
 * design — when a new endpoint lands, the matching tool is added here
 * in the same PR.
 *
 * Tool handlers read the authenticated user / token from
 * `extra.authInfo.extra` (populated by withMcpAuth + verifyClaudepotToken).
 *
 * Validation: each tool defines a per-field inputSchema for the LLM
 * client (this is what shows up in tools/list), then runs the SAME
 * shared Zod schema (e.g. submissionInputSchema) inside the handler
 * to enforce object-level rules (URL XOR text) the per-field schema
 * cannot express. Skipping the shared schema in the past meant MCP
 * accepted both/neither URL+text while REST rejected the same input.
 */

import type { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { z } from "zod";

import { checkAndIncrement } from "@/lib/api/rate-limit";
import type { Scope } from "@/lib/api/scopes";
import {
  createSubmission,
  submissionInputSchema,
  SUBMISSION_TYPES,
} from "@/lib/submissions";
import type { ClaudepotAuthExtra } from "./auth";

function getAuthExtra(
  extra: { authInfo?: { extra?: Record<string, unknown> } },
): ClaudepotAuthExtra | null {
  const ai = extra.authInfo?.extra;
  if (
    !ai ||
    typeof ai.userId !== "string" ||
    typeof ai.username !== "string" ||
    typeof ai.role !== "string" ||
    typeof ai.tokenId !== "string" ||
    typeof ai.tokenPrefix !== "string"
  ) {
    return null;
  }
  return ai as unknown as ClaudepotAuthExtra;
}

function hasScope(
  extra: { authInfo?: { scopes?: string[] } },
  scope: Scope,
): boolean {
  return extra.authInfo?.scopes?.includes(scope) ?? false;
}

function textResult(text: string, isError = false) {
  return {
    isError,
    content: [{ type: "text" as const, text }],
  };
}

export function registerTools(server: McpServer): void {
  /* ── submit_link ──────────────────────────────────────────────
   *
   * Creates a submission. Maps 1:1 to POST /api/v1/submissions.
   * Requires the submission:write scope; rate-limited per token.
   */

  server.registerTool(
    "submit_link",
    {
      title: "Submit a link to sha.com",
      description:
        "Create a new submission on sha.com. Provide either `url` " +
        "(for link posts) OR `text` (for self-posts) — never both, never " +
        "neither. Requires the submission:write scope. Outcome (auto-" +
        "publish vs AI moderation queue) depends on the user's role and " +
        "karma.",
      inputSchema: {
        type: z
          .enum(SUBMISSION_TYPES)
          .describe(
            "Submission type. Use 'discussion' for self-posts (text only). All other types should normally have a url.",
          ),
        title: z
          .string()
          .min(3)
          .max(120)
          .describe("Submission title (3-120 chars, will be trimmed)."),
        url: z
          .string()
          .url()
          .optional()
          .describe(
            "External URL. Provide url XOR text — both or neither will be rejected.",
          ),
        text: z
          .string()
          .max(40_000)
          .optional()
          .describe(
            "Self-post body, markdown. Provide url XOR text — both or neither will be rejected.",
          ),
        tags: z
          .array(z.string())
          .max(5)
          .optional()
          .describe("Up to 5 tag slugs (kebab-case)."),
      },
    },
    async (args, extra) => {
      const auth = getAuthExtra(extra);
      if (!auth) return textResult("Unauthorized: missing or invalid token.", true);

      if (!hasScope(extra, "submission:write")) {
        return textResult(
          "Forbidden: this token is missing the submission:write scope.",
          true,
        );
      }

      // Object-level validation (URL XOR text, etc.) that the per-field
      // inputSchema cannot express. REST and web go through the same
      // schema; running it here keeps MCP behavior in lockstep.
      const parsed = submissionInputSchema.safeParse(args);
      if (!parsed.success) {
        const issues = parsed.error.issues
          .map((i) => `${i.path.join(".") || "<root>"}: ${i.message}`)
          .join("; ");
        return textResult(`Validation failed: ${issues}`, true);
      }

      const limit = await checkAndIncrement(auth.tokenId, "submissions");
      if (!limit.ok) {
        return textResult(
          `Rate limited: daily submission limit (${limit.limit}) exceeded. Resets at ${limit.resetAt.toISOString()}.`,
          true,
        );
      }

      const result = await createSubmission(auth.userId, parsed.data, {
        surface: "api",
        tokenId: auth.tokenId,
        tokenPrefix: auth.tokenPrefix,
      });

      if (!result.ok) {
        if (result.reason === "duplicate") {
          return textResult(
            `Duplicate URL — already submitted in the last 30 days. Existing submission: https://claudepot.com/post/${result.existingId}`,
          );
        }
        if (result.reason === "locked") {
          return textResult("Forbidden: account is locked.", true);
        }
        return textResult(
          `Validation failed: ${result.detail ?? "unknown error"}`,
          true,
        );
      }

      const url = `https://claudepot.com/post/${result.submissionId}`;
      return textResult(
        result.pending
          ? `Submitted (id ${result.submissionId}) — routed to AI moderation. ${url}`
          : `Published: ${url}`,
      );
    },
  );

  /* ── me ───────────────────────────────────────────────────────
   *
   * Token introspection. Maps 1:1 to GET /api/v1/me. No scope required;
   * any active token can call it. Useful for MCP clients to verify
   * connectivity + see what they can do.
   */

  server.registerTool(
    "me",
    {
      title: "Identify the calling user",
      description:
        "Return the username, role, and granted scopes for the token used to authenticate. " +
        "No scope required — any active token can call it.",
      inputSchema: {},
    },
    async (_args, extra) => {
      const auth = getAuthExtra(extra);
      if (!auth) return textResult("Unauthorized.", true);
      const scopes = extra.authInfo?.scopes ?? [];
      return textResult(
        JSON.stringify(
          {
            username: auth.username,
            role: auth.role,
            scopes,
            tokenPrefix: auth.tokenPrefix,
          },
          null,
          2,
        ),
      );
    },
  );
}
