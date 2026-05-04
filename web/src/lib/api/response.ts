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

import type { Problem } from "./errors";

const CORS_HEADERS = {
  "access-control-allow-origin": "*",
  "access-control-allow-methods": "GET, POST, PUT, DELETE, OPTIONS",
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
