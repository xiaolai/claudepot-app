/**
 * MCP tool — bot self-reporting.
 *
 * Mirrors POST /api/v1/bots/reports. Single tool with a `kind`
 * discriminator instead of six per-kind tools — the operator
 * decision (admin-redesign.md, P5) was that one mega-tool keeps
 * the manifest simple at the cost of slightly worse LLM
 * discoverability. Re-split if the bot-side ergonomics suffer.
 */

import type { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { z } from "zod";

import { persistBotReport, REPORT_KINDS, reportInputSchema } from "@/lib/bots";
import { chargeForTool, checkAuthForTool } from "../policy";
import { formatZodIssues, textResult } from "./helpers";

export function registerBotWriteTools(server: McpServer): void {
  server.registerTool(
    "report_bot_status",
    {
      title: "Post a bot self-report (heartbeat / work / cost / error / proposal / decision_summary)",
      description:
        "Posts a self-report for the bot identity that owns this token. " +
        "bot_id is derived from the token — there is no bot_id field. " +
        "Kind discriminates the payload schema:\n" +
        "  • heartbeat: { version?, env?, meta? } — UPSERTs liveness; " +
        "does not consume the rate-limit budget.\n" +
        "  • work_summary: { windowStart, windowEnd, units: Record<string,int>, notes? } — " +
        "rolled-up batch of work units in the window.\n" +
        "  • cost: { provider, model, usd, inputTokens?, outputTokens?, notes? } — " +
        "usd is denormalized to bot_reports.cost_usd for fast roll-up.\n" +
        "  • error: { severity: 'warn'|'error', message, context? } — " +
        "non-fatal but operator-worthy.\n" +
        "  • proposal: { kind: 'vocab_tag'|'block_user'|'tag_merge'|'tag_retire'|'general', " +
        "reason, target?, key? } — " +
        "surfaces in the /admin Today inbox notice strip until staff ack. " +
        "Pass a stable `key` for retry-idempotency; re-posts with the same " +
        "(bot_id, key) while still open are rejected as duplicates.\n" +
        "  • decision_summary: { windowStart, windowEnd, verdicts, confidence?, driftZ?, notes? } — " +
        "moderation-class drift telemetry.\n" +
        "Requires the bots:report scope.",
      inputSchema: {
        kind: z.enum(REPORT_KINDS),
        payload: z.record(z.string(), z.unknown()),
        /** Optional explicit cost override; otherwise derived from payload.usd on cost reports. */
        costUsd: z.number().nonnegative().optional(),
      },
    },
    async (args, extra) => {
      const a = await checkAuthForTool("report_bot_status", extra);
      if (!a.ok) return a.result;

      const parsed = reportInputSchema.safeParse(args);
      if (!parsed.success) {
        return textResult(
          `Validation failed: ${formatZodIssues(parsed.error)}`,
          true,
        );
      }

      // Heartbeats UPSERT a single row and are deliberately cheap.
      // Mirrors the REST handler's skip-charge branch.
      if (parsed.data.kind !== "heartbeat") {
        const c = await chargeForTool("report_bot_status", a.ctx.tokenId);
        if (!c.ok) return c.result;
      }

      const result = await persistBotReport(a.ctx.userId, parsed.data);
      if (!result.ok) {
        if (result.reason === "validation") {
          return textResult(
            `Payload validation failed: ${result.detail}`,
            true,
          );
        }
        if (result.reason === "duplicate") {
          return textResult(
            "Conflict: a proposal with this dedup key is already open for " +
              "this bot. Wait for staff to resolve it or post under a " +
              "different payload.key.",
            true,
          );
        }
        return textResult("Bot report failed.", true);
      }

      if (result.kind === "heartbeat") {
        return textResult(
          `Heartbeat received. last_seen_at=${result.lastSeenAt.toISOString()}`,
        );
      }
      return textResult(
        `Recorded ${result.kind} report ${result.reportId}.`,
      );
    },
  );
}
