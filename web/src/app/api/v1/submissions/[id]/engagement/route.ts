/**
 * GET /api/v1/submissions/{id}/engagement — engagement event log
 * for a submission.
 *
 * Reads engagement_records (added in 0036_editorial_writes), the
 * minimal event log referenced from editorial/transparency.md but
 * not previously built. Public read (read:all). Cursor-free —
 * caps at 500 most-recent events ordered by occurredAt desc, with
 * an optional `since` filter.
 *
 * `kind` is a free-form text field on the table (matches the
 * applied_persona convention). Filter via `kind` query param,
 * comma-separated for multi-value.
 */

import { and, desc, eq, gte, inArray } from "drizzle-orm";

import { db } from "@/db/client";
import { engagementRecords, submissions } from "@/db/schema";
import { notFound, validation } from "@/lib/api/errors";
import { ok, preflight, problemResponse, withErrorHandling } from "@/lib/api/response";
import { isUuid } from "@/lib/api/inputs";
import { endpointSpec } from "@/lib/api/manifest";
import { chargeForSpec, checkAuthForSpec } from "@/lib/api/policy";

const HARD_LIMIT = 500;

export async function OPTIONS(): Promise<Response> {
  return preflight();
}

export const GET = withErrorHandling(async (
  req: Request,
  { params }: { params: Promise<{ id: string }> },
): Promise<Response> => {
  const { id } = await params;
  if (!isUuid(id)) return problemResponse(notFound("Invalid id."));

  const SPEC = endpointSpec("submissions:list_engagement");
  const policy = await checkAuthForSpec(req, SPEC);
  if (!policy.ok) return policy.response;
  const { auth } = policy;

  const url = new URL(req.url);
  const sinceRaw = url.searchParams.get("since");
  const kindRaw = url.searchParams.get("kind");
  let since: Date | null = null;
  if (sinceRaw) {
    const t = new Date(sinceRaw);
    if (Number.isNaN(t.getTime())) {
      return problemResponse(
        validation("Query param 'since' must be an ISO-8601 timestamp."),
      );
    }
    since = t;
  }
  const kinds = kindRaw
    ? kindRaw
        .split(",")
        .map((s) => s.trim())
        .filter((s) => s.length > 0)
    : [];

  const charge = await chargeForSpec(SPEC, auth.token.id);
  if (!charge.ok) return charge.response;

  const [sub] = await db
    .select({ id: submissions.id })
    .from(submissions)
    .where(eq(submissions.id, id))
    .limit(1);
  if (!sub) return problemResponse(notFound("Submission not found."));

  const filters = [eq(engagementRecords.submissionId, id)];
  if (since) filters.push(gte(engagementRecords.occurredAt, since));
  if (kinds.length > 0) filters.push(inArray(engagementRecords.kind, kinds));

  // Privacy: actor_id is recorded but NEVER returned. "User X
  // voted on submission Y at time T" would leak per-user voting
  // history; vote counts are public, identities are not. metadata
  // is also dropped — the backing JSONB is unconstrained and a
  // future private payload would become public the moment it lands.
  const rows = await db
    .select({
      id: engagementRecords.id,
      kind: engagementRecords.kind,
      occurredAt: engagementRecords.occurredAt,
    })
    .from(engagementRecords)
    .where(and(...filters))
    .orderBy(desc(engagementRecords.occurredAt))
    .limit(HARD_LIMIT);

  return ok({
    submissionId: id,
    events: rows.map((r) => ({
      id: r.id,
      kind: r.kind,
      occurredAt: r.occurredAt.toISOString(),
    })),
  });
});
