/**
 * MCP tool definitions for sha.com.
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
 * The 12 read tools live in lib/mcp/read-tools.ts and are registered
 * via registerReadTools() below; this file owns the writes plus the
 * identity / introspection / constitution tools.
 */

import type { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { z } from "zod";

import {
  commentInputSchema,
  createComment,
  deleteCommentAsAuthor,
  updateCommentAsAuthor,
  updateCommentInputSchema,
} from "@/lib/comments";
import {
  createSubmission,
  deleteSubmissionAsAuthor,
  submissionInputSchema,
  SUBMISSION_TYPES,
  updateSubmissionAsAuthor,
  updateSubmissionInputSchema,
} from "@/lib/submissions";
import {
  listNotificationsForUser,
  listNotificationsInputSchema,
  markNotificationsReadForUser,
  markReadInputSchema,
  NOTIFICATION_KINDS,
} from "@/lib/notifications";
import { castVote, saveInputSchema, setSave, voteInputSchema } from "@/lib/votes";
import { getConstitution } from "@/lib/api/constitution";
import { readQuotaForToken } from "@/lib/api/quota";
import { chargeForTool, checkAuthForTool } from "./policy";
import { registerReadTools } from "./read-tools";

function textResult(text: string, isError = false) {
  return { isError, content: [{ type: "text" as const, text }] };
}

// Format a Zod safeParse error into the inline text the tool result
// surfaces back to the LLM client.
function formatZodIssues(error: z.ZodError): string {
  return error.issues
    .map((i) => `${i.path.join(".") || "<root>"}: ${i.message}`)
    .join("; ");
}

export function registerTools(server: McpServer): void {
  // Read tools live in lib/mcp/read-tools.ts so this file stays
  // focused on the writes + identity surface.
  registerReadTools(server);

  /* ── submit_link ──────────────────────────────────────────────
   *
   * Creates a submission. Maps 1:1 to POST /api/v1/submissions.
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
        title: z.string().min(3).max(120),
        url: z.string().url().optional().describe("Provide url XOR text."),
        text: z.string().max(40_000).optional(),
        tags: z.array(z.string()).max(5).optional(),
      },
    },
    async (args, extra) => {
      const a = await checkAuthForTool("submit_link", extra);
      if (!a.ok) return a.result;

      const parsed = submissionInputSchema.safeParse(args);
      if (!parsed.success) {
        return textResult(
          `Validation failed: ${formatZodIssues(parsed.error)}`,
          true,
        );
      }

      const c = await chargeForTool("submit_link", a.ctx.tokenId);
      if (!c.ok) return c.result;

      const result = await createSubmission(a.ctx.userId, parsed.data, {
        surface: "api",
        tokenId: a.ctx.tokenId,
        tokenPrefix: a.ctx.tokenPrefix,
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

  /* ── delete_submission ─────────────────────────────────────── */
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
        submissionId: z.uuid(),
      },
    },
    async (args, extra) => {
      const a = await checkAuthForTool("delete_submission", extra);
      if (!a.ok) return a.result;
      const c = await chargeForTool("delete_submission", a.ctx.tokenId);
      if (!c.ok) return c.result;
      const result = await deleteSubmissionAsAuthor(
        a.ctx.userId,
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

  /* ── update_submission ─────────────────────────────────────── */
  server.registerTool(
    "update_submission",
    {
      title: "Edit one of your own submissions",
      description:
        "Updates the title and/or text of a submission you authored. " +
        "Provide at least one of title / text. URL is intentionally " +
        "not editable (it carries dedup identity). The post must " +
        "remain a link post (url set, no text) OR a self-post (no " +
        "url, text set). Requires the submission:update scope.\n\n" +
        "Window policy:\n" +
        "  Authorization — humans (role=user, is_agent=false) only " +
        "within 5 minutes of posting; bots (is_agent) and platform " +
        "users (system / staff) any time.\n" +
        "  Visibility — within-window edits are SILENT; out-of-window " +
        "edits set updated_at and the UI shows an 'edited' badge.",
      inputSchema: {
        submissionId: z.uuid(),
        title: z.string().min(3).max(120).optional(),
        text: z.string().max(40_000).optional(),
      },
    },
    async (args, extra) => {
      const a = await checkAuthForTool("update_submission", extra);
      if (!a.ok) return a.result;

      const { submissionId, ...rest } = args;
      const parsed = updateSubmissionInputSchema.safeParse(rest);
      if (!parsed.success) {
        return textResult(
          `Validation failed: ${formatZodIssues(parsed.error)}`,
          true,
        );
      }

      const c = await chargeForTool("update_submission", a.ctx.tokenId);
      if (!c.ok) return c.result;

      const result = await updateSubmissionAsAuthor(
        a.ctx.userId,
        submissionId,
        parsed.data,
      );
      if (!result.ok) {
        if (result.reason === "forbidden") {
          return textResult(
            "Forbidden: you can only edit your own submissions.",
            true,
          );
        }
        if (result.reason === "expired") {
          return textResult(
            "Forbidden: edit window expired. Bot tokens (is_agent / system / staff) bypass it.",
            true,
          );
        }
        if (result.reason === "invalid") {
          return textResult(
            `Invalid: ${result.detail ?? "Update would violate the URL/text invariant."}`,
            true,
          );
        }
        if (result.reason === "noop") {
          return textResult(`No-op: nothing changed on ${submissionId}.`);
        }
        return textResult("Submission not found.", true);
      }
      const badge = result.silent ? "silent" : "visible (edited badge shown)";
      return textResult(`Edited submission ${submissionId} — ${badge}.`);
    },
  );

  /* ── post_comment ──────────────────────────────────────────── */
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
        submissionId: z.uuid(),
        parentId: z.uuid().nullable().optional(),
        body: z.string().min(2).max(40_000),
      },
    },
    async (args, extra) => {
      const a = await checkAuthForTool("post_comment", extra);
      if (!a.ok) return a.result;

      const parsed = commentInputSchema.safeParse(args);
      if (!parsed.success) {
        return textResult(
          `Validation failed: ${formatZodIssues(parsed.error)}`,
          true,
        );
      }

      const c = await chargeForTool("post_comment", a.ctx.tokenId);
      if (!c.ok) return c.result;

      const result = await createComment(a.ctx.userId, parsed.data);
      if (!result.ok) {
        if (result.reason === "not_found") {
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

  /* ── update_comment ────────────────────────────────────────── */
  server.registerTool(
    "update_comment",
    {
      title: "Edit one of your own comments",
      description:
        "Updates the body of a comment you authored. Requires the " +
        "comment:update scope. Same window policy as update_submission.",
      inputSchema: {
        commentId: z.uuid(),
        body: z.string().min(2).max(40_000),
      },
    },
    async (args, extra) => {
      const a = await checkAuthForTool("update_comment", extra);
      if (!a.ok) return a.result;

      const { commentId, ...rest } = args;
      const parsed = updateCommentInputSchema.safeParse(rest);
      if (!parsed.success) {
        return textResult(
          `Validation failed: ${formatZodIssues(parsed.error)}`,
          true,
        );
      }

      const c = await chargeForTool("update_comment", a.ctx.tokenId);
      if (!c.ok) return c.result;

      const result = await updateCommentAsAuthor(
        a.ctx.userId,
        commentId,
        parsed.data,
      );
      if (!result.ok) {
        if (result.reason === "forbidden") {
          return textResult(
            "Forbidden: you can only edit your own comments.",
            true,
          );
        }
        if (result.reason === "expired") {
          return textResult(
            "Forbidden: edit window expired. Bot tokens (is_agent / system / staff) bypass it.",
            true,
          );
        }
        if (result.reason === "noop") {
          return textResult(`No-op: nothing changed on ${commentId}.`);
        }
        return textResult("Comment not found.", true);
      }
      const badge = result.silent ? "silent" : "visible (edited badge shown)";
      return textResult(`Edited comment ${commentId} — ${badge}.`);
    },
  );

  /* ── delete_comment ────────────────────────────────────────── */
  server.registerTool(
    "delete_comment",
    {
      title: "Delete one of your own comments",
      description:
        "Deletes a comment you authored. If the comment has replies, the " +
        "row is soft-deleted (tombstone preserved); otherwise it is " +
        "hard-deleted. Requires the comment:delete scope.",
      inputSchema: {
        commentId: z.uuid(),
      },
    },
    async (args, extra) => {
      const a = await checkAuthForTool("delete_comment", extra);
      if (!a.ok) return a.result;
      const c = await chargeForTool("delete_comment", a.ctx.tokenId);
      if (!c.ok) return c.result;
      const result = await deleteCommentAsAuthor(a.ctx.userId, args.commentId);
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

  /* ── vote ──────────────────────────────────────────────────── */
  server.registerTool(
    "vote",
    {
      title: "Vote on a submission",
      description:
        "Cast (1), reverse (-1), or clear (0) a vote on the given " +
        "submission. Downvotes require karma >= 100 (staff exempt). " +
        "Requires the vote:write scope.",
      inputSchema: {
        submissionId: z.uuid(),
        value: z.union([z.literal(1), z.literal(-1), z.literal(0)]),
      },
    },
    async (args, extra) => {
      const a = await checkAuthForTool("vote", extra);
      if (!a.ok) return a.result;

      const parsed = voteInputSchema.safeParse(args);
      if (!parsed.success) {
        return textResult(
          `Validation failed: ${formatZodIssues(parsed.error)}`,
          true,
        );
      }

      const c = await chargeForTool("vote", a.ctx.tokenId);
      if (!c.ok) return c.result;

      const result = await castVote(a.ctx.userId, parsed.data);
      if (!result.ok) {
        if (result.reason === "karma_gate") {
          return textResult("Forbidden: downvotes require at least 100 karma.", true);
        }
        if (result.reason === "locked") {
          return textResult("Forbidden: account is locked.", true);
        }
        if (result.reason === "missing_user") {
          return textResult("Unauthorized: token references a deleted user.", true);
        }
        return textResult("Submission not found, or not in a votable state.", true);
      }
      return textResult(
        `Vote recorded: submission ${parsed.data.submissionId}, value ${result.value}.`,
      );
    },
  );

  /* ── save ──────────────────────────────────────────────────── */
  server.registerTool(
    "save",
    {
      title: "Save (or unsave) a submission",
      description:
        "Adds or removes a private bookmark on the given submission. " +
        "Idempotent. Requires the save:write scope.",
      inputSchema: {
        submissionId: z.uuid(),
        saved: z.boolean(),
      },
    },
    async (args, extra) => {
      const a = await checkAuthForTool("save", extra);
      if (!a.ok) return a.result;

      const parsed = saveInputSchema.safeParse(args);
      if (!parsed.success) {
        return textResult(
          `Validation failed: ${formatZodIssues(parsed.error)}`,
          true,
        );
      }

      const c = await chargeForTool("save", a.ctx.tokenId);
      if (!c.ok) return c.result;

      const result = await setSave(a.ctx.userId, parsed.data);
      if (!result.ok) {
        return textResult("Submission not found, or not in a saveable state.", true);
      }
      return textResult(
        `${parsed.data.saved ? "Saved" : "Unsaved"} submission ${parsed.data.submissionId}.`,
      );
    },
  );

  /* ── list_notifications ────────────────────────────────────── */
  server.registerTool(
    "list_notifications",
    {
      title: "List your notifications",
      description:
        "Returns the calling user's notifications, newest first. " +
        "Use `since` to do incremental polling. Requires the " +
        "notification:read scope.",
      inputSchema: {
        unreadOnly: z.boolean().optional(),
        since: z.iso.datetime().optional(),
        limit: z.number().int().min(1).max(200).optional(),
        kinds: z
          .array(z.enum(NOTIFICATION_KINDS))
          .max(NOTIFICATION_KINDS.length)
          .optional(),
      },
    },
    async (args, extra) => {
      const a = await checkAuthForTool("list_notifications", extra);
      if (!a.ok) return a.result;

      const parsed = listNotificationsInputSchema.safeParse(args);
      if (!parsed.success) {
        return textResult(
          `Validation failed: ${formatZodIssues(parsed.error)}`,
          true,
        );
      }

      const c = await chargeForTool("list_notifications", a.ctx.tokenId);
      if (!c.ok) return c.result;

      const result = await listNotificationsForUser(a.ctx.userId, parsed.data);
      return textResult(JSON.stringify(result, null, 2));
    },
  );

  /* ── mark_notifications_read ──────────────────────────────── */
  server.registerTool(
    "mark_notifications_read",
    {
      title: "Mark notifications as read",
      description:
        "Marks notifications as read for the calling user. Pass " +
        "`ids` to mark specific items, or `all: true` to mark every " +
        "unread item. Idempotent. Requires the notification:read scope.",
      inputSchema: {
        ids: z.array(z.uuid()).max(500).optional(),
        all: z.boolean().optional(),
      },
    },
    async (args, extra) => {
      const a = await checkAuthForTool("mark_notifications_read", extra);
      if (!a.ok) return a.result;

      const parsed = markReadInputSchema.safeParse(args);
      if (!parsed.success) {
        return textResult(
          `Validation failed: ${formatZodIssues(parsed.error)}`,
          true,
        );
      }

      const c = await chargeForTool("mark_notifications_read", a.ctx.tokenId);
      if (!c.ok) return c.result;

      const result = await markNotificationsReadForUser(a.ctx.userId, parsed.data);
      return textResult(`Marked ${result.updated} notification(s) as read.`);
    },
  );

  /* ── get_constitution ─────────────────────────────────────── */
  server.registerTool(
    "get_constitution",
    {
      title: "Read the editorial constitution",
      description:
        "Returns the public editorial sources (audience, rubric, " +
        "transparency) plus a stable `version` string. The " +
        "`rubric.public` field is the structured public-safe view " +
        "— weights, thresholds, and persona multipliers are " +
        "intentionally omitted. Requires read:all.",
      inputSchema: {},
    },
    async (_args, extra) => {
      const a = await checkAuthForTool("get_constitution", extra);
      if (!a.ok) return a.result;
      const c = await chargeForTool("get_constitution", a.ctx.tokenId);
      if (!c.ok) return c.result;
      const constitution = getConstitution();
      return textResult(JSON.stringify(constitution, null, 2));
    },
  );

  /* ── get_quota ────────────────────────────────────────────── */
  server.registerTool(
    "get_quota",
    {
      title: "Read the calling token's daily quota",
      description:
        "Returns the daily usage and limits for each rate-limited " +
        "category (submissions, comments, votes, saves, reads), " +
        "along with the reset timestamp. No scope required and no " +
        "rate-limit charge.",
      inputSchema: {},
    },
    async (_args, extra) => {
      const a = await checkAuthForTool("get_quota", extra);
      if (!a.ok) return a.result;
      const quota = await readQuotaForToken(a.ctx.tokenId);
      return textResult(JSON.stringify(quota, null, 2));
    },
  );

  /* ── me ──────────────────────────────────────────────────── */
  server.registerTool(
    "me",
    {
      title: "Identify the calling user",
      description:
        "Return the username, role, and granted scopes for the token " +
        "used to authenticate. No scope required.",
      inputSchema: {},
    },
    async (_args, extra) => {
      const a = await checkAuthForTool("me", extra);
      if (!a.ok) return a.result;
      const scopes = extra.authInfo?.scopes ?? [];
      return textResult(
        JSON.stringify(
          {
            username: a.ctx.username,
            role: a.ctx.role,
            scopes,
            tokenPrefix: a.ctx.tokenPrefix,
          },
          null,
          2,
        ),
      );
    },
  );
}
