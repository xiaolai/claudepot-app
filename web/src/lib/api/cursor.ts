/**
 * Opaque cursor encoding for /api/v1/* list endpoints.
 *
 * The cursor is a base64url-encoded JSON object that captures the
 * tail of the last page so the next page can resume after it without
 * needing the client to understand the encoding. Two shapes:
 *
 *   { t: epochMs, id: uuid }   — time-ordered (sort=new). Page boundary
 *                                is (createdAt, id).
 *   { s: score,   id: uuid }   — score-ordered (sort=top). Page boundary
 *                                is (score, id).
 *
 * The id is a tiebreaker for rows that share a sort key — without it,
 * a page boundary would loop forever on a tied score / millisecond.
 *
 * Decoding is total — invalid input returns null, never throws.
 * Routes treat null as "no cursor" rather than 422, which means a
 * stale-but-format-valid cursor is silently ignored on the next page;
 * a corrupted cursor is rejected by the route as a 400/422 explicitly.
 */

const UUID_RE =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

export type CursorTime = { t: number; id: string };
export type CursorScore = { s: number; id: string };
export type Cursor = CursorTime | CursorScore;

export function encodeCursor(c: Cursor): string {
  // Buffer / base64url is Node 16+; the project's engines.node = 24.
  return Buffer.from(JSON.stringify(c), "utf-8").toString("base64url");
}

/**
 * Returns null on any decode failure: malformed base64, invalid JSON,
 * wrong shape, non-uuid id, non-finite numeric. The caller decides
 * whether to surface a 422 or to silently treat the cursor as missing.
 */
export function decodeCursor(s: string | null | undefined): Cursor | null {
  if (!s) return null;
  let json: string;
  try {
    json = Buffer.from(s, "base64url").toString("utf-8");
  } catch {
    return null;
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(json);
  } catch {
    return null;
  }
  if (parsed === null || typeof parsed !== "object") return null;
  const o = parsed as Record<string, unknown>;
  if (typeof o.id !== "string" || !UUID_RE.test(o.id)) return null;

  if ("t" in o) {
    if (typeof o.t !== "number" || !Number.isFinite(o.t)) return null;
    return { t: o.t, id: o.id };
  }
  if ("s" in o) {
    if (typeof o.s !== "number" || !Number.isFinite(o.s)) return null;
    return { s: o.s, id: o.id };
  }
  return null;
}

export function isCursorTime(c: Cursor): c is CursorTime {
  return "t" in c;
}

export function isCursorScore(c: Cursor): c is CursorScore {
  return "s" in c;
}

/* ── Limit clamp ─────────────────────────────────────────────────
 *
 * Default 50, max 200 per the PRD. Non-finite / non-positive values
 * fall back to the default rather than 422 — feed reads should be
 * permissive about a misbehaving client's `limit` param.
 */

export const DEFAULT_PAGE_LIMIT = 50;
export const MAX_PAGE_LIMIT = 200;

export function clampPageLimit(input: unknown): number {
  if (typeof input !== "number" || !Number.isFinite(input) || input <= 0) {
    return DEFAULT_PAGE_LIMIT;
  }
  return Math.min(Math.floor(input), MAX_PAGE_LIMIT);
}
