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

import {
  checkAndIncrement,
  type LimitCategory,
} from "@/lib/api/rate-limit";
import type { Scope } from "@/lib/api/scopes";
import { commentInputSchema, createComment, deleteCommentAsAuthor } from "@/lib/comments";
import {
  createSubmission,
  deleteSubmissionAsAuthor,
  submissionInputSchema,
  SUBMISSION_TYPES,
} from "@/lib/submissions";
import { castVote, saveInputSchema, setSave, voteInputSchema } from "@/lib/votes";
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

// Format a Zod safeParse error into the inline text the tool result
// surfaces back to the LLM client.
function formatZodIssues(error: z.ZodError): string {
  return error.issues
    .map((i) => `${i.path.join(".") || "<root>"}: ${i.message}`)
    .join("; ");
}

// Singular noun used in rate-limit messages. Keeps the user-facing
// wording stable across categories ("daily comment limit", not
// "daily comments limit") instead of leaking the plural column name.
const RATE_LIMIT_NOUN: Record<LimitCategory, string> = {
  submissions: "submission",
  comments: "comment",
  votes: "vote",
  saves: "save",
  reads: "read",
};

// Bumps the rate-limit bucket for `category` and returns the matching
// "Rate limited" textResult on overflow, or null on success. Returning
// null lets call sites stay flat: `const limited = …; if (limited)
// return limited;`.
async function enforceRateLimit(
  tokenId: string,
  category: LimitCategory,
): Promise<ReturnType<typeof textResult> | null> {
  const limit = await checkAndIncrement(tokenId, category);
  if (limit.ok) return null;
  return textResult(
    `Rate limited: daily ${RATE_LIMIT_NOUN[category]} limit (${limit.limit}) exceeded. Resets at ${limit.resetAt.toISOString()}.`,
    true,
  );
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
        return textResult(
          `Validation failed: ${formatZodIssues(parsed.error)}`,
          true,
        );
      }

      const limited = await enforceRateLimit(auth.tokenId, "submissions");
      if (limited) return limited;

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

  /* ── delete_submission ─────────────────────────────────────────
   *
   * Author-only soft delete. Maps 1:1 to DELETE /api/v1/submissions/:id.
   * Requires the submission:delete scope.
   */

  server.registerTool(
    "delete_submission",
    {
      title: "Delete one of your own submissions",
      description:
        "Soft-deletes a submission you authored. Requires the " +
        "submission:delete scope. Counts against the daily submission " +
        "budget. Returns an error if the submission is not yours or no " +
        "longer exists.",
      inputSchema: {
        submissionId: z
          .uuid()
          .describe("UUID of the submission to delete (must be yours)."),
      },
    },
    async (args, extra) => {
      const auth = getAuthExtra(extra);
      if (!auth) return textResult("Unauthorized.", true);

      if (!hasScope(extra, "submission:delete")) {
        return textResult(
          "Forbidden: this token is missing the submission:delete scope.",
          true,
        );
      }

      const limited = await enforceRateLimit(auth.tokenId, "submissions");
      if (limited) return limited;

      const result = await deleteSubmissionAsAuthor(
        auth.userId,
        args.submissionId,
      );
      if (!result.ok) {
        if (result.reason === "forbidden") {
          return textResult(
            "Forbidden: you can only delete your own submissions.",
            true,
          );
        }
        return textResult("Submission not found.", true);
      }
      return textResult(`Deleted submission ${args.submissionId}.`);
    },
  );

  /* ── post_comment ──────────────────────────────────────────────
   *
   * Posts a top-level comment or reply. Maps 1:1 to POST /api/v1/comments.
   * Requires the comment:write scope.
   */

  server.registerTool(
    "post_comment",
    {
      title: "Post a comment or reply on a submission",
      description:
        "Posts a comment on the given submission. Pass parentId to make " +
        "it a reply. Returns the new comment id and whether it was " +
        "auto-published (depends on the user's role and karma). Requires " +
        "the comment:write scope.",
      inputSchema: {
        submissionId: z.uuid().describe("UUID of the parent submission."),
        parentId: z
          .uuid()
          .nullable()
          .optional()
          .describe(
            "Optional UUID of a comment in the same submission to reply to.",
          ),
        body: z
          .string()
          .min(2)
          .max(40_000)
          .describe("Comment markdown body (2-40000 chars, will be trimmed)."),
      },
    },
    async (args, extra) => {
      const auth = getAuthExtra(extra);
      if (!auth) return textResult("Unauthorized.", true);

      if (!hasScope(extra, "comment:write")) {
        return textResult(
          "Forbidden: this token is missing the comment:write scope.",
          true,
        );
      }

      const parsed = commentInputSchema.safeParse(args);
      if (!parsed.success) {
        return textResult(
          `Validation failed: ${formatZodIssues(parsed.error)}`,
          true,
        );
      }

      const limited = await enforceRateLimit(auth.tokenId, "comments");
      if (limited) return limited;

      const result = await createComment(auth.userId, parsed.data);
      if (!result.ok) {
        if (result.reason === "not_found") {
          // Covers a missing/deleted submission AND a missing /
          // deleted / rejected / pending parent comment. Tightening
          // the wording would leak which one — keep generic.
          return textResult("Submission or parent comment not found.", true);
        }
        if (result.reason === "locked") {
          return textResult(
            "Forbidden: your account is locked, or the submission is closed to new comments.",
            true,
          );
        }
        return textResult("Comment failed.", true);
      }

      const url = `https://claudepot.com/post/${parsed.data.submissionId}#comment-${result.commentId}`;
      return textResult(
        result.pending
          ? `Comment ${result.commentId} routed to AI moderation. ${url}`
          : `Posted: ${url}`,
      );
    },
  );

  /* ── delete_comment ────────────────────────────────────────────
   *
   * Author-only delete (soft when the comment has replies, hard
   * otherwise). Maps 1:1 to DELETE /api/v1/comments/:id.
   */

  server.registerTool(
    "delete_comment",
    {
      title: "Delete one of your own comments",
      description:
        "Deletes a comment you authored. If the comment has replies, the " +
        "row is soft-deleted (tombstone preserved); otherwise it is " +
        "hard-deleted. Requires the comment:delete scope. Counts against " +
        "the daily comment budget.",
      inputSchema: {
        commentId: z
          .uuid()
          .describe("UUID of the comment to delete (must be yours)."),
      },
    },
    async (args, extra) => {
      const auth = getAuthExtra(extra);
      if (!auth) return textResult("Unauthorized.", true);

      if (!hasScope(extra, "comment:delete")) {
        return textResult(
          "Forbidden: this token is missing the comment:delete scope.",
          true,
        );
      }

      const limited = await enforceRateLimit(auth.tokenId, "comments");
      if (limited) return limited;

      const result = await deleteCommentAsAuthor(auth.userId, args.commentId);
      if (!result.ok) {
        if (result.reason === "forbidden") {
          return textResult(
            "Forbidden: you can only delete your own comments.",
            true,
          );
        }
        return textResult("Comment not found.", true);
      }
      return textResult(`Deleted comment ${args.commentId}.`);
    },
  );

  /* ── vote ──────────────────────────────────────────────────────
   *
   * Cast / change / clear a vote. Maps 1:1 to POST /api/v1/votes.
   * Requires the vote:write scope.
   */

  server.registerTool(
    "vote",
    {
      title: "Vote on a submission",
      description:
        "Cast (1), reverse (-1), or clear (0) a vote on the given " +
        "submission. Downvotes require karma >= 100 (staff exempt). " +
        "Requires the vote:write scope.",
      inputSchema: {
        submissionId: z.uuid().describe("UUID of the submission."),
        value: z
          .union([z.literal(1), z.literal(-1), z.literal(0)])
          .describe("1 = upvote, -1 = downvote, 0 = clear vote."),
      },
    },
    async (args, extra) => {
      const auth = getAuthExtra(extra);
      if (!auth) return textResult("Unauthorized.", true);

      if (!hasScope(extra, "vote:write")) {
        return textResult(
          "Forbidden: this token is missing the vote:write scope.",
          true,
        );
      }

      const parsed = voteInputSchema.safeParse(args);
      if (!parsed.success) {
        return textResult(
          `Validation failed: ${formatZodIssues(parsed.error)}`,
          true,
        );
      }

      const limited = await enforceRateLimit(auth.tokenId, "votes");
      if (limited) return limited;

      const result = await castVote(auth.userId, parsed.data);
      if (!result.ok) {
        if (result.reason === "karma_gate") {
          return textResult(
            "Forbidden: downvotes require at least 100 karma.",
            true,
          );
        }
        if (result.reason === "locked") {
          return textResult("Forbidden: account is locked.", true);
        }
        if (result.reason === "missing_user") {
          return textResult("Unauthorized: token references a deleted user.", true);
        }
        return textResult(
          "Submission not found, or not in a votable state.",
          true,
        );
      }

      return textResult(
        `Vote recorded: submission ${parsed.data.submissionId}, value ${result.value}.`,
      );
    },
  );

  /* ── save ──────────────────────────────────────────────────────
   *
   * Toggle a private bookmark. Maps 1:1 to POST /api/v1/saves.
   * Requires the save:write scope.
   */

  server.registerTool(
    "save",
    {
      title: "Save (or unsave) a submission",
      description:
        "Adds or removes a private bookmark on the given submission. " +
        "Idempotent — duplicate saves and missing unsaves are absorbed. " +
        "Requires the save:write scope.",
      inputSchema: {
        submissionId: z.uuid().describe("UUID of the submission."),
        saved: z
          .boolean()
          .describe("true = bookmark, false = remove bookmark."),
      },
    },
    async (args, extra) => {
      const auth = getAuthExtra(extra);
      if (!auth) return textResult("Unauthorized.", true);

      if (!hasScope(extra, "save:write")) {
        return textResult(
          "Forbidden: this token is missing the save:write scope.",
          true,
        );
      }

      const parsed = saveInputSchema.safeParse(args);
      if (!parsed.success) {
        return textResult(
          `Validation failed: ${formatZodIssues(parsed.error)}`,
          true,
        );
      }

      const limited = await enforceRateLimit(auth.tokenId, "saves");
      if (limited) return limited;

      const result = await setSave(auth.userId, parsed.data);
      if (!result.ok) {
        return textResult(
          "Submission not found, or not in a saveable state.",
          true,
        );
      }
      return textResult(
        `${parsed.data.saved ? "Saved" : "Unsaved"} submission ${parsed.data.submissionId}.`,
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
