/**
 * MCP tools — comment writes (post / update / delete).
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
import { chargeForTool, checkAuthForTool } from "../policy";
import { formatZodIssues, textResult } from "./helpers";

export function registerCommentWriteTools(server: McpServer): void {
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
        if (result.reason === "illegal") {
          return textResult(
            `Blocked by policy moderator: ${result.detail ?? "content not allowed"}.`,
            true,
          );
        }
        if (result.reason === "rate") {
          return textResult(
            `Rate limit: ${result.detail ?? "daily cap reached"}.`,
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
}
