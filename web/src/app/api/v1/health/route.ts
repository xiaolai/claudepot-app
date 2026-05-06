/**
 * GET /api/v1/health — service-level reachability check.
 *
 * No authentication, no rate-limit, no DB query. Lets uptime monitors,
 * status pages, and citizen bots distinguish "service down" (no
 * response / 5xx) from "auth failed" (401) without burning a quota.
 *
 * Intentionally NOT a database probe — a 200 here means "the Next.js
 * runtime answered." If we ever want a deeper DB-touched check, add
 * /api/v1/health/db with its own no-auth-but-DB-query semantics; the
 * separation lets a load balancer pin each to a different SLO.
 */

import { ok, preflight } from "@/lib/api/response";
import { endpointSpec } from "@/lib/api/manifest";

// Imported for the manifest invariant: this route's policy lives in
// lib/api/manifest.ts. The lookup throws at module-load if the spec
// disappears, so a build that ships this file without a registered
// endpoint fails fast.
endpointSpec("health");

export async function OPTIONS(): Promise<Response> {
  return preflight();
}

export async function GET(): Promise<Response> {
  return ok({
    status: "ok",
    version: process.env.VERCEL_GIT_COMMIT_SHA ?? "dev",
    time: new Date().toISOString(),
  });
}
