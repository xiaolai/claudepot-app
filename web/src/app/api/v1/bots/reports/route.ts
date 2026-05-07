/**
 * POST /api/v1/bots/reports — bot self-reporting endpoint.
 *
 * One endpoint, six kinds:
 *   { kind: "heartbeat",        payload: { version?, env?, meta? } }
 *   { kind: "work_summary",     payload: { windowStart, windowEnd, units, notes? } }
 *   { kind: "cost",             payload: { provider, model, usd, … } }
 *   { kind: "error",            payload: { severity, message, context? } }
 *   { kind: "proposal",         payload: { kind, reason, target?, key? } }
 *   { kind: "decision_summary", payload: { windowStart, windowEnd, verdicts, … } }
 *
 * `bot_id` is derived from the authenticated token's user — there
 * is no bot_id field in the body. A leaked token therefore can
 * only post for the one bot it belongs to. Per-bot isolation
 * without a per-bot scope name.
 *
 * heartbeat skips the rate-limit charge (UPSERT one row, not
 * load-bearing). Every other kind charges the `bots` bucket.
 *
 * Proposals are deduped by a partial unique index on
 * (bot_id, payload->>'key') WHERE status='open'. Re-posting the
 * same proposal while one is open returns 409 — surface the
 * duplicate to the bot so it doesn't retry forever.
 */

import { conflict, validation } from "@/lib/api/errors";
import { ok, preflight, problemResponse } from "@/lib/api/response";
import { endpointSpec } from "@/lib/api/manifest";
import { chargeForSpec, checkAuthForSpec } from "@/lib/api/policy";
import { persistBotReport, reportInputSchema } from "@/lib/bots";

export async function OPTIONS(): Promise<Response> {
  return preflight();
}

export async function POST(req: Request): Promise<Response> {
  const SPEC = endpointSpec("bots:report");
  const policy = await checkAuthForSpec(req, SPEC);
  if (!policy.ok) return policy.response;
  const { auth } = policy;

  let body: unknown;
  try {
    body = await req.json();
  } catch {
    return problemResponse(validation("Request body must be valid JSON."));
  }

  const parsed = reportInputSchema.safeParse(body);
  if (!parsed.success) {
    return problemResponse(
      validation(
        "Bot report validation failed.",
        parsed.error.issues.map((i) => ({
          field: i.path.join(".") || "(root)",
          message: i.message,
        })),
      ),
    );
  }

  // Heartbeats skip the rate-limit charge. They UPSERT a single row
  // and are deliberately cheap so bots can ping aggressively
  // without burning their daily budget.
  if (parsed.data.kind !== "heartbeat") {
    const charge = await chargeForSpec(SPEC, auth.token.id);
    if (!charge.ok) return charge.response;
  }

  const result = await persistBotReport(auth.user.id, parsed.data);

  if (!result.ok) {
    if (result.reason === "validation") {
      return problemResponse(
        validation("Bot report payload validation failed.", [
          { field: "payload", message: result.detail },
        ]),
      );
    }
    if (result.reason === "duplicate") {
      return problemResponse(
        conflict(
          "A proposal with this dedup key is already open for this bot. " +
            "Wait for staff to resolve it, or post under a different `payload.key`.",
        ),
      );
    }
    // Exhaustiveness: PersistResult only has these failure shapes.
    // If a new one is added, this branch should narrow to never.
    const _exhaustive: never = result;
    void _exhaustive;
    return problemResponse(validation("Bot report failed."));
  }

  if (result.kind === "heartbeat") {
    return ok({
      kind: "heartbeat" as const,
      botId: auth.user.id,
      lastSeenAt: result.lastSeenAt.toISOString(),
    });
  }
  return ok({
    kind: result.kind,
    botId: auth.user.id,
    reportId: result.reportId,
  });
}
