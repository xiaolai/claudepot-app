/**
 * MCP tools — submission writes (create / update / delete).
 *
 * Each tool wraps the matching lib/submissions function. Auth +
 * scope + bucket policy comes from the manifest via lib/mcp/policy.ts.
 */

import type { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { z } from "zod";

import {
  createSubmission,
  deleteSubmissionAsAuthor,
  submissionInputSchema,
  SUBMISSION_TYPES,
  updateSubmissionAsAuthor,
  updateSubmissionInputSchema,
} from "@/lib/submissions";
import { chargeForTool, checkAuthForTool } from "../policy";
import { formatZodIssues, textResult } from "./helpers";

export function registerSubmissionWriteTools(server: McpServer): void {
  /* ── submit_link ──────────────────────────────────────────────
   *
   * Creates a submission. Maps 1:1 to POST /api/v1/submissions.
   */
  server.registerTool(
    "submit_link",
    {
      title: "Submit a link to claudepot.com",
      description:
        "Create a new submission on claudepot.com. Provide either `url` " +
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
        if (result.reason === "rate") {
          return textResult(
            `Rate limit: ${result.detail ?? "daily cap reached"}.`,
            true,
          );
        }
        if (result.reason === "rejected") {
          const appealLine = result.decisionId
            ? ` Appeal: https://claudepot.com/appeal/${result.decisionId}`
            : " (Audit record could not be written; contact staff to appeal.)";
          return textResult(
            `Blocked by policy moderator (${result.category}): ${result.oneLineWhy}.${appealLine}`,
            true,
          );
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
}
