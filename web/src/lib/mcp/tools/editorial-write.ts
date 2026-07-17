/**
 * MCP tools — editorial-runtime writes (migration 0036).
 *
 * Five tools mirror the office-only REST endpoints under
 * /api/v1/decisions, /scout-runs, /engagement, and
 * /submissions/{id}/publish. Auth + scope + bucket policy comes
 * from the manifest via lib/mcp/policy.ts.
 *
 * Citizens never see these in tools/list with a read-only token —
 * the scopes are granted to office bots only. The tool definitions
 * exist to honor the "every REST endpoint has a 1:1 MCP tool"
 * contract documented on /api.
 */

import type { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { z } from "zod";

import {
  decisionInputSchema,
  engagementMetadataSchema,
  overrideInputSchema,
  persistDecision,
  persistOverride,
  persistScoutRun,
  scoutRunInputSchema,
} from "@/lib/editorial-writes";
import { recordEngagement } from "@/lib/engagement";
import { publishSubmission } from "@/lib/submissions";
import { db } from "@/db/client";
import { submissions } from "@/db/schema";
import { eq } from "drizzle-orm";

import { chargeForTool, checkAuthForTool } from "../policy";
import { formatZodIssues, textResult } from "./helpers";

export function registerEditorialWriteTools(server: McpServer): void {
  /* ── write_decision ──────────────────────────────────────────
   * POST /api/v1/decisions. Idempotent on
   * (submissionId, appliedPersona, modelId, promptHash).
   * Pure storage — never touches submissions.state. */
  server.registerTool(
    "write_decision",
    {
      title: "Write an editorial scoring decision",
      description:
        "Records one editorial decision_records row. Idempotent: " +
        "re-POSTing the same (submissionId, appliedPersona, modelId, " +
        "promptHash) returns the existing id. NEVER touches " +
        "submissions.state — call publish_submission separately when " +
        "the office's policy says yes. appliedPersona and " +
        "perCriterionScores keys are free-form text/jsonb on the " +
        "polity side; the office owns the vocabulary.",
      inputSchema: {
        submissionId: z.uuid(),
        rubricVersion: z.string().min(1).max(80),
        audienceDocVersion: z.string().min(1).max(80),
        appliedPersona: z.string().min(1).max(80),
        perCriterionScores: z.record(z.string(), z.number()),
        weightedTotal: z.number(),
        hardRejectsHit: z.array(z.string()).optional(),
        inclusionGates: z.record(z.string(), z.boolean()),
        typeInferred: z.string(),
        subSegmentInferred: z.string().min(1).max(120),
        confidence: z.enum(["high", "low"]),
        oneLineWhy: z.string().min(1).max(2000),
        finalDecision: z.enum(["accept", "reject", "borderline_to_human_queue"]),
        routing: z.enum(["feed", "firehose", "human_queue"]),
        modelId: z.string().min(1).max(120),
        promptHash: z.string().min(1).max(120).optional(),
        costUsd: z.number().nonnegative().max(1000).optional(),
      },
    },
    async (args, extra) => {
      const a = await checkAuthForTool("write_decision", extra);
      if (!a.ok) return a.result;

      const parsed = decisionInputSchema.safeParse(args);
      if (!parsed.success) {
        return textResult(
          `Validation failed: ${formatZodIssues(parsed.error)}`,
          true,
        );
      }

      const c = await chargeForTool("write_decision", a.ctx.tokenId);
      if (!c.ok) return c.result;

      const result = await persistDecision(parsed.data);
      if (!result.ok) {
        if (result.reason === "submission_not_found") {
          return textResult("Submission not found.", true);
        }
        return textResult(`Validation: ${result.detail}`, true);
      }
      return textResult(
        JSON.stringify({
          id: result.decisionId,
          created: result.created,
        }),
      );
    },
  );

  /* ── override_decision ───────────────────────────────────────
   * POST /api/v1/decisions/{id}/override. reviewer_kind='bot'
   * forced. Pure storage — never touches submissions.state. */
  server.registerTool(
    "override_decision",
    {
      title: "Override an existing editorial decision",
      description:
        "Files an override_records row against an existing " +
        "decision_records row. reviewer_kind is forced to 'bot'. " +
        "NEVER touches submissions.state — call publish_submission " +
        "separately if the override should publish (or unpublish) the " +
        "submission. The original decision is preserved as part of " +
        "the audit trail.",
      inputSchema: {
        decisionId: z.uuid(),
        overrideDecision: z.enum([
          "accept",
          "reject",
          "borderline_to_human_queue",
        ]),
        overrideRouting: z.enum(["feed", "firehose", "human_queue"]),
        reviewerScores: z.record(z.string(), z.number()).optional(),
        reason: z.string().min(1).max(2000),
      },
    },
    async (args, extra) => {
      const a = await checkAuthForTool("override_decision", extra);
      if (!a.ok) return a.result;

      const { decisionId, ...rest } = args;
      const parsed = overrideInputSchema.safeParse(rest);
      if (!parsed.success) {
        return textResult(
          `Validation failed: ${formatZodIssues(parsed.error)}`,
          true,
        );
      }

      const c = await chargeForTool("override_decision", a.ctx.tokenId);
      if (!c.ok) return c.result;

      const result = await persistOverride(
        decisionId as string,
        a.ctx.userId,
        parsed.data,
      );
      if (!result.ok) {
        if (result.reason === "decision_not_found") {
          return textResult("Decision not found.", true);
        }
        return textResult(`Validation: ${result.detail}`, true);
      }
      return textResult(JSON.stringify({ id: result.overrideId }));
    },
  );

  /* ── record_scout_run ────────────────────────────────────────
   * POST /api/v1/scout-runs. Counts only — per-source extraction
   * rules stay private per editorial/transparency.md §3. */
  server.registerTool(
    "record_scout_run",
    {
      title: "Record a scout-pass aggregate (counts only)",
      description:
        "Logs a single scout invocation's aggregate counts to " +
        "scout_runs. The polity stores counts; per-source extraction " +
        "rules and source-specific adapters stay private inside the " +
        "office repo. Validation refuses inverted timestamps and " +
        "itemsKept + itemsDropped > itemsPulled.",
      inputSchema: {
        sourceId: z.string().min(1).max(120),
        startedAt: z.iso.datetime(),
        finishedAt: z.iso.datetime(),
        itemsPulled: z.number().int().nonnegative(),
        itemsKept: z.number().int().nonnegative(),
        itemsDropped: z.number().int().nonnegative(),
        error: z.string().max(2000).optional(),
      },
    },
    async (args, extra) => {
      const a = await checkAuthForTool("record_scout_run", extra);
      if (!a.ok) return a.result;

      const parsed = scoutRunInputSchema.safeParse(args);
      if (!parsed.success) {
        return textResult(
          `Validation failed: ${formatZodIssues(parsed.error)}`,
          true,
        );
      }

      const c = await chargeForTool("record_scout_run", a.ctx.tokenId);
      if (!c.ok) return c.result;

      const result = await persistScoutRun(parsed.data);
      if (!result.ok) {
        return textResult(`Validation: ${result.detail}`, true);
      }
      return textResult(JSON.stringify({ id: result.scoutRunId }));
    },
  );

  /* ── publish_submission ──────────────────────────────────────
   * POST /api/v1/submissions/{id}/publish. Flip
   * draft↔approved. Bot-author only; idempotent. */
  server.registerTool(
    "publish_submission",
    {
      title: "Publish or unpublish a draft submission",
      description:
        "Flips an office-controlled submission between state='draft' " +
        "and 'approved'. publish=true sets publishedAt; publish=false " +
        "clears it. Idempotent — re-call returns outcome='unchanged'. " +
        "Refuses citizen-authored submissions (those go through Ada / " +
        "staff). Refuses 'pending' or 'rejected' rows — those aren't " +
        "part of the office's draft↔approved cycle.",
      inputSchema: {
        submissionId: z.uuid(),
        publish: z
          .boolean()
          .describe(
            "true: draft→approved (publish). false: approved→draft (retract).",
          ),
      },
    },
    async (args, extra) => {
      const a = await checkAuthForTool("publish_submission", extra);
      if (!a.ok) return a.result;

      // Mirror the REST handler's bot gate: this primitive is for
      // bot accounts only, even with the right scope.
      if (!a.ctx.isAgent) {
        return textResult(
          "publish_submission is callable only from bot accounts (users.is_agent=true).",
          true,
        );
      }

      const c = await chargeForTool("publish_submission", a.ctx.tokenId);
      if (!c.ok) return c.result;

      const result = await publishSubmission(
        args.submissionId as string,
        Boolean(args.publish),
      );
      if (!result.ok) {
        if (result.reason === "submission_not_found") {
          return textResult("Submission not found.", true);
        }
        if (result.reason === "not_office_owned") {
          return textResult(
            result.detail ??
              "Publish primitive is only valid on bot-authored submissions.",
            true,
          );
        }
        return textResult(
          result.detail ?? "Submission is not in a transitionable state.",
          true,
        );
      }
      return textResult(
        JSON.stringify({
          submissionId: args.submissionId,
          outcome: result.outcome,
          state: result.state,
        }),
      );
    },
  );

  /* ── record_engagement ───────────────────────────────────────
   * POST /api/v1/engagement. Office-defined semantic kinds.
   * Refuses primitive kinds (vote/comment/save) to prevent
   * double-counting against the polity's auto-recording. */
  server.registerTool(
    "record_engagement",
    {
      title: "Record an office-defined semantic engagement event",
      description:
        "Append a higher-level engagement event (e.g. " +
        "'discussion_started', 'topic_drift_detected', " +
        "'cross_referenced'). Primitive events ('vote', 'comment', " +
        "'save') are auto-recorded by the polity on the corresponding " +
        "handlers — passing those kinds here is rejected. The polity " +
        "stores the event verbatim; the office owns the vocabulary.",
      inputSchema: {
        submissionId: z.uuid(),
        kind: z
          .string()
          .min(1)
          .max(80)
          .describe(
            "Office-defined kind. Avoid 'vote', 'comment', 'save' — those are auto-recorded.",
          ),
        // Serialized size capped at 4 KB — shared with the REST
        // twin via engagementMetadataSchema.
        metadata: engagementMetadataSchema.optional(),
      },
    },
    async (args, extra) => {
      const a = await checkAuthForTool("record_engagement", extra);
      if (!a.ok) return a.result;

      const kind = String(args.kind ?? "");
      if (kind === "vote" || kind === "comment" || kind === "save") {
        return textResult(
          "Primitive kinds ('vote', 'comment', 'save') are recorded automatically by the polity — use a semantic kind here.",
          true,
        );
      }
      if (kind.length === 0 || kind.length > 80) {
        return textResult("kind must be 1-80 chars.", true);
      }
      // Explicit re-check of the metadata size cap, mirroring the
      // manual kind checks above rather than trusting the SDK's
      // inputSchema validation alone.
      if (args.metadata !== undefined) {
        const meta = engagementMetadataSchema.safeParse(args.metadata);
        if (!meta.success) {
          return textResult(
            `Validation failed: ${formatZodIssues(meta.error)}`,
            true,
          );
        }
      }

      const c = await chargeForTool("record_engagement", a.ctx.tokenId);
      if (!c.ok) return c.result;

      const submissionId = args.submissionId as string;
      const [sub] = await db
        .select({ id: submissions.id })
        .from(submissions)
        .where(eq(submissions.id, submissionId))
        .limit(1);
      if (!sub) return textResult("Submission not found.", true);

      await recordEngagement({
        submissionId,
        kind,
        actorId: a.ctx.userId,
        metadata: args.metadata as Record<string, unknown> | undefined,
      });

      return textResult(JSON.stringify({ recorded: true }));
    },
  );
}
