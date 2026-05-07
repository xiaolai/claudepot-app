/**
 * MCP tools — vote + save (engagement).
 */

import type { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { z } from "zod";

import { castVote, saveInputSchema, setSave, voteInputSchema } from "@/lib/votes";
import { chargeForTool, checkAuthForTool } from "../policy";
import { formatZodIssues, textResult } from "./helpers";

export function registerVoteSaveTools(server: McpServer): void {
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
}
