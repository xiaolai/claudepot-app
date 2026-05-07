/**
 * Response factories. Success = `{ data: ... }`, failure = problem+json.
 *
 * CORS is open (`*`) on every response. Authorization triggers a CORS
 * preflight, which we explicitly allow via Access-Control-Allow-Headers,
 * so any origin's JS that holds a token can call the API directly. This
 * is fine because (a) the threat model treats tokens as the credential —
 * the client origin doesn't matter, and (b) credentials: include is NOT
 * permitted alongside allow-origin: *, so cookies cannot be carried
 * cross-origin even by accident. Token leakage is the relevant risk to
 * mitigate, and that lives outside CORS.
 */

import { internal, type Problem } from "./errors";

const CORS_HEADERS = {
  "access-control-allow-origin": "*",
  // PATCH is the verb used for partial updates (submissions, comments);
  // no route uses PUT today. Listing PATCH here lets browsers complete
  // the cross-origin preflight for /api/v1/submissions/{id} edits.
  "access-control-allow-methods": "GET, POST, PATCH, DELETE, OPTIONS",
  "access-control-allow-headers": "authorization, content-type",
  "access-control-max-age": "86400",
};

const JSON_HEADERS = {
  "content-type": "application/json; charset=utf-8",
  ...CORS_HEADERS,
};

const PROBLEM_HEADERS = {
  "content-type": "application/problem+json; charset=utf-8",
  ...CORS_HEADERS,
};

export function ok<T>(data: T, init: ResponseInit = {}): Response {
  return new Response(JSON.stringify({ data }), {
    status: 200,
    ...init,
    headers: { ...JSON_HEADERS, ...(init.headers ?? {}) },
  });
}

export function created<T>(data: T, location?: string): Response {
  const headers: Record<string, string> = { ...JSON_HEADERS };
  if (location) headers.location = location;
  return new Response(JSON.stringify({ data }), { status: 201, headers });
}

export function noContent(): Response {
  return new Response(null, { status: 204, headers: CORS_HEADERS });
}

/**
 * 304 Not Modified — for ETag revalidation. The body MUST be empty
 * (RFC 7232 §4.1) and the ETag header SHOULD be re-sent so the client
 * can refresh its cache key without parsing the body.
 */
export function notModified(etag: string): Response {
  return new Response(null, {
    status: 304,
    headers: { ...CORS_HEADERS, etag },
  });
}

export function problemResponse(p: Problem): Response {
  return new Response(JSON.stringify(p), {
    status: p.status,
    headers: PROBLEM_HEADERS,
  });
}

/** OPTIONS preflight handler — every route exports this. */
export function preflight(): Response {
  return new Response(null, { status: 204, headers: CORS_HEADERS });
}

/**
 * Wrap a route handler so an unhandled exception returns a structured
 * problem+json 500 instead of Next's default HTML error page. Logs the
 * underlying error to stderr; never exposes it in the response body.
 *
 * Usage:
 *   export const POST = withErrorHandling(async (req) => { ... });
 */
type RouteHandler<Args extends unknown[]> = (
  ...args: Args
) => Promise<Response> | Response;

export function withErrorHandling<Args extends unknown[]>(
  handler: RouteHandler<Args>,
): RouteHandler<Args> {
  return async (...args: Args): Promise<Response> => {
    try {
      return await handler(...args);
    } catch (err) {
      console.error("[api] unhandled error", err);
      return problemResponse(internal());
    }
  };
}
