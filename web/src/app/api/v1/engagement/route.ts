/**
 * POST /api/v1/engagement — append an office-defined semantic
 * engagement event.
 *
 * Primitive events ('vote', 'comment', 'save') are recorded
 * automatically by the polity on the corresponding handlers — the
 * office should NOT call this endpoint to log a vote / comment /
 * save. Use this for higher-level interpretations:
 *
 *   - 'discussion_started' — the bot detected a thread reaching N
 *     replies after a long quiet period.
 *   - 'topic_drift_detected' — Layer-3 EIC noticed the comments
 *     diverged from the submission's topic.
 *   - 'cross_referenced' — another piece on the feed cited this one.
 *
 * The polity stores the event verbatim; it doesn't validate or
 * interpret the kind. New office-defined kinds land without a polity
 * migration. Body shape: { submissionId, kind, metadata? }.
 *
 * actor_id is taken from the authenticated user (the office bot
 * making the call). Citizens never reach this endpoint —
 * engagement:write is granted to bot accounts only.
 */

import { z } from "zod";

import { notFound, validation } from "@/lib/api/errors";
import { eq } from "drizzle-orm";

import { db } from "@/db/client";
import { submissions } from "@/db/schema";
import {
  created,
  preflight,
  problemResponse,
  withErrorHandling,
} from "@/lib/api/response";
import { endpointSpec } from "@/lib/api/manifest";
import { chargeForSpec, checkAuthForSpec } from "@/lib/api/policy";
import { recordEngagement } from "@/lib/engagement";

const engagementInputSchema = z.object({
  submissionId: z.uuid(),
  // Open vocabulary: free-form text identifying the event kind. The
  // polity DOES reject the primitive kinds (vote/comment/save) here
  // to prevent the office from accidentally double-recording them.
  kind: z
    .string()
    .min(1)
    .max(80)
    .refine((v) => v !== "vote" && v !== "comment" && v !== "save", {
      message:
        "Primitive kinds ('vote', 'comment', 'save') are recorded automatically by the polity — use a semantic kind here.",
    }),
  metadata: z.record(z.string(), z.unknown()).optional(),
});

export async function OPTIONS(): Promise<Response> {
  return preflight();
}

export const POST = withErrorHandling(async (req: Request): Promise<Response> => {
  const SPEC = endpointSpec("engagement:create");
  const policy = await checkAuthForSpec(req, SPEC);
  if (!policy.ok) return policy.response;
  const { auth } = policy;

  let body: unknown;
  try {
    body = await req.json();
  } catch {
    return problemResponse(validation("Request body must be valid JSON."));
  }

  const parsed = engagementInputSchema.safeParse(body);
  if (!parsed.success) {
    return problemResponse(
      validation(
        "Engagement validation failed.",
        parsed.error.issues.map((i) => ({
          field: i.path.join(".") || "(root)",
          message: i.message,
        })),
      ),
    );
  }

  const charge = await chargeForSpec(SPEC, auth.token.id);
  if (!charge.ok) return charge.response;

  const [sub] = await db
    .select({ id: submissions.id })
    .from(submissions)
    .where(eq(submissions.id, parsed.data.submissionId))
    .limit(1);
  if (!sub) return problemResponse(notFound("Submission not found."));

  // recordEngagement is best-effort by design (callers in the vote /
  // comment / save handlers don't await for correctness). Here we
  // DO want the call to surface failure to the API caller, so we
  // call db.insert directly with explicit error handling.
  await recordEngagement({
    submissionId: parsed.data.submissionId,
    kind: parsed.data.kind,
    actorId: auth.user.id,
    metadata: parsed.data.metadata,
  });

  return created({ recorded: true });
});
