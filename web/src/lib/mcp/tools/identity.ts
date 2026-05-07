/**
 * MCP tools — identity / introspection / constitution.
 *
 *   - get_constitution: public editorial sources.
 *   - get_quota: per-token daily usage.
 *   - list_my_decisions: caller's own AI policy moderator decisions.
 *   - me: token introspection (username, role, scopes).
 */

import type { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { z } from "zod";

import { getConstitution } from "@/lib/api/constitution";
import { readQuotaForToken } from "@/lib/api/quota";
import {
  getMyDecision,
  getMyDecisionInputSchema,
  listMyDecisions,
  listMyDecisionsInputSchema,
} from "@/lib/moderation";
import { chargeForTool, checkAuthForTool } from "../policy";
import { formatZodIssues, textResult } from "./helpers";

export function registerIdentityTools(server: McpServer): void {
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

  /* ── list_my_decisions ───────────────────────────────────── */
  server.registerTool(
    "list_my_decisions",
    {
      title: "List your AI policy moderator decisions",
      description:
        "Returns the calling user's own AI policy moderator decisions, " +
        "newest first. Use `since` to incrementally poll. Filter `kind` " +
        "to scope to submission or comment decisions. Requires read:all.",
      inputSchema: {
        kind: z.enum(["submission", "comment"]).optional(),
        since: z.iso.datetime().optional(),
      },
    },
    async (args, extra) => {
      const a = await checkAuthForTool("list_my_decisions", extra);
      if (!a.ok) return a.result;

      const parsed = listMyDecisionsInputSchema.safeParse(args);
      if (!parsed.success) {
        return textResult(
          `Validation failed: ${formatZodIssues(parsed.error)}`,
          true,
        );
      }

      const c = await chargeForTool("list_my_decisions", a.ctx.tokenId);
      if (!c.ok) return c.result;

      const result = await listMyDecisions(a.ctx.userId, parsed.data);
      return textResult(JSON.stringify(result, null, 2));
    },
  );

  /* ── get_my_decision ─────────────────────────────────────── */
  server.registerTool(
    "get_my_decision",
    {
      title: "Read one of your AI policy moderator decisions",
      description:
        "Returns a single AI policy moderator decision by id. Cross-" +
        "user access returns not_found (decision-id existence is not " +
        "disclosed). Requires read:all.",
      inputSchema: {
        id: z.uuid(),
      },
    },
    async (args, extra) => {
      const a = await checkAuthForTool("get_my_decision", extra);
      if (!a.ok) return a.result;

      const parsed = getMyDecisionInputSchema.safeParse(args);
      if (!parsed.success) {
        return textResult(
          `Validation failed: ${formatZodIssues(parsed.error)}`,
          true,
        );
      }

      const c = await chargeForTool("get_my_decision", a.ctx.tokenId);
      if (!c.ok) return c.result;

      const decision = await getMyDecision(a.ctx.userId, parsed.data);
      if (!decision) {
        return textResult("Decision not found.", true);
      }
      return textResult(JSON.stringify(decision, null, 2));
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
