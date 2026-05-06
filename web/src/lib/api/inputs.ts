/**
 * Query-string parsing helpers shared across /api/v1/* read endpoints.
 *
 * Returns a discriminated `{ ok, ... }` so route handlers can either
 * forward the parsed values into a query helper or return a 422 with
 * the offending field name. The cursor is kept opaque — callers
 * receive a typed `Cursor | null`, never the raw base64.
 */

import { decodeCursor, type Cursor } from "./cursor";
import { SUBMISSION_TYPES } from "@/lib/submissions";
import type { SubmissionType } from "./dto";

const UUID_RE =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
const USERNAME_RE = /^[a-z0-9_-]{1,32}$/i;
const TAG_SLUG_RE = /^[a-z0-9-]{1,40}$/;

/** Path-param UUID guard. */
export function isUuid(s: string): boolean {
  return UUID_RE.test(s);
}

/** Path-param username guard — matches the citext column shape. */
export function isUsername(s: string): boolean {
  return USERNAME_RE.test(s);
}

export type ParseError = { field: string; message: string };

export type ParseResult<T> =
  | { ok: true; value: T }
  | { ok: false; errors: ParseError[] };

function parseLimit(raw: string | null): ParseResult<number | undefined> {
  if (raw === null) return { ok: true, value: undefined };
  const n = Number(raw);
  // Reject fractional values explicitly — the prior `!Number.isFinite`
  // accepted "12.5" and silently floored it, which contradicts the
  // "Must be a positive integer" message. isInteger implies finite.
  if (!Number.isInteger(n) || n <= 0) {
    return {
      ok: false,
      errors: [{ field: "limit", message: "Must be a positive integer." }],
    };
  }
  if (n > 200) {
    return {
      ok: false,
      errors: [{ field: "limit", message: "Maximum is 200." }],
    };
  }
  return { ok: true, value: n };
}

function parseSince(raw: string | null): ParseResult<Date | null> {
  if (raw === null) return { ok: true, value: null };
  const d = new Date(raw);
  if (Number.isNaN(d.getTime())) {
    return {
      ok: false,
      errors: [
        {
          field: "since",
          message: "Must be a valid ISO 8601 timestamp.",
        },
      ],
    };
  }
  return { ok: true, value: d };
}

function parseCursorParam(raw: string | null): ParseResult<Cursor | null> {
  if (raw === null) return { ok: true, value: null };
  const c = decodeCursor(raw);
  if (c === null) {
    // Format-invalid cursors are surfaced — clients should pass exactly
    // what `nextCursor` returned. A stale-but-format-valid cursor that
    // mismatches the current sort key is silently ignored downstream.
    return {
      ok: false,
      errors: [{ field: "cursor", message: "Invalid cursor." }],
    };
  }
  return { ok: true, value: c };
}

/* ── Submission list params ──────────────────────────────────────── */

export type SubmissionListParams = {
  sort: "new" | "top";
  cursor: Cursor | null;
  limit: number | undefined;
  since: Date | null;
  types: SubmissionType[] | null;
  tagSlugs: string[] | null;
  authorUsername: string | null;
  state: "approved" | "pending";
};

export function parseSubmissionListParams(
  url: URL,
): ParseResult<SubmissionListParams> {
  const errors: ParseError[] = [];

  const sortRaw = url.searchParams.get("sort");
  let sort: "new" | "top" = "new";
  if (sortRaw !== null) {
    if (sortRaw === "new" || sortRaw === "top") sort = sortRaw;
    else if (sortRaw === "controversial") {
      // Documented in the PRD as v0-skip. Treat as "new" rather than
      // 422 so a forward-looking client doesn't break on rollout.
      sort = "new";
    } else {
      errors.push({
        field: "sort",
        message: "Must be one of: new, top, controversial.",
      });
    }
  }

  const limit = parseLimit(url.searchParams.get("limit"));
  if (!limit.ok) errors.push(...limit.errors);

  const since = parseSince(url.searchParams.get("since"));
  if (!since.ok) errors.push(...since.errors);

  const cursor = parseCursorParam(url.searchParams.get("cursor"));
  if (!cursor.ok) errors.push(...cursor.errors);

  const types: SubmissionType[] = [];
  for (const t of url.searchParams.getAll("type")) {
    if (!(SUBMISSION_TYPES as readonly string[]).includes(t)) {
      errors.push({ field: "type", message: `Unknown submission type: ${t}.` });
    } else {
      types.push(t as SubmissionType);
    }
  }

  const tagSlugs: string[] = [];
  for (const t of url.searchParams.getAll("tag")) {
    if (!TAG_SLUG_RE.test(t)) {
      errors.push({ field: "tag", message: `Invalid tag slug: ${t}.` });
    } else {
      tagSlugs.push(t);
    }
  }

  let authorUsername: string | null = null;
  const author = url.searchParams.get("author");
  if (author !== null) {
    if (!isUsername(author)) {
      errors.push({
        field: "author",
        message: "Invalid username.",
      });
    } else {
      authorUsername = author;
    }
  }

  let state: "approved" | "pending" = "approved";
  const stateRaw = url.searchParams.get("state");
  if (stateRaw !== null) {
    if (stateRaw === "approved" || stateRaw === "pending") state = stateRaw;
    else {
      errors.push({
        field: "state",
        message: "Must be one of: approved, pending.",
      });
    }
  }

  if (errors.length > 0) return { ok: false, errors };

  return {
    ok: true,
    value: {
      sort,
      cursor: cursor.ok ? cursor.value : null,
      limit: limit.ok ? limit.value : undefined,
      since: since.ok ? since.value : null,
      types: types.length > 0 ? types : null,
      tagSlugs: tagSlugs.length > 0 ? tagSlugs : null,
      authorUsername,
      state,
    },
  };
}

/* ── Comment list params ─────────────────────────────────────────── */

export type CommentListParams = {
  cursor: Cursor | null;
  limit: number | undefined;
  since: Date | null;
  depth: number;
};

export function parseCommentListParams(
  url: URL,
): ParseResult<CommentListParams> {
  const errors: ParseError[] = [];

  const limit = parseLimit(url.searchParams.get("limit"));
  if (!limit.ok) errors.push(...limit.errors);
  const since = parseSince(url.searchParams.get("since"));
  if (!since.ok) errors.push(...since.errors);
  const cursor = parseCursorParam(url.searchParams.get("cursor"));
  if (!cursor.ok) errors.push(...cursor.errors);

  let depth = 5;
  const depthRaw = url.searchParams.get("depth");
  if (depthRaw !== null) {
    const n = Number(depthRaw);
    if (!Number.isFinite(n) || n <= 0 || n > 20) {
      errors.push({
        field: "depth",
        message: "Must be a positive integer ≤ 20.",
      });
    } else {
      depth = Math.floor(n);
    }
  }

  if (errors.length > 0) return { ok: false, errors };
  return {
    ok: true,
    value: {
      cursor: cursor.ok ? cursor.value : null,
      limit: limit.ok ? limit.value : undefined,
      since: since.ok ? since.value : null,
      depth,
    },
  };
}

/* ── Search params ───────────────────────────────────────────────── */

export type SearchParams = {
  q: string;
  kind: "submission" | "comment";
  cursor: Cursor | null;
  limit: number | undefined;
  since: Date | null;
  types: SubmissionType[] | null;
  tagSlugs: string[] | null;
  authorUsername: string | null;
};

export function parseSearchParams(url: URL): ParseResult<SearchParams> {
  const errors: ParseError[] = [];

  const q = (url.searchParams.get("q") ?? "").trim();
  if (q.length < 2 || q.length > 200) {
    errors.push({
      field: "q",
      message: "Must be 2–200 characters.",
    });
  }

  let kind: "submission" | "comment" = "submission";
  const kindRaw = url.searchParams.get("kind");
  if (kindRaw !== null) {
    if (kindRaw === "submission" || kindRaw === "comment") kind = kindRaw;
    else {
      errors.push({
        field: "kind",
        message: "Must be one of: submission, comment.",
      });
    }
  }

  // Reuse submission-list parsing for the shared filters.
  const subParams = parseSubmissionListParams(url);
  if (!subParams.ok) {
    // Strip out fields not in scope for search (sort, state).
    for (const e of subParams.errors) {
      if (e.field === "sort" || e.field === "state") continue;
      errors.push(e);
    }
  }

  if (errors.length > 0) return { ok: false, errors };
  if (!subParams.ok) {
    // Should be unreachable — we just handed back errors.
    return { ok: false, errors: [{ field: "_", message: "Parse failed." }] };
  }

  return {
    ok: true,
    value: {
      q,
      kind,
      cursor: subParams.value.cursor,
      limit: subParams.value.limit,
      since: subParams.value.since,
      types: subParams.value.types,
      tagSlugs: subParams.value.tagSlugs,
      authorUsername: subParams.value.authorUsername,
    },
  };
}

/* ── Tag list params ─────────────────────────────────────────────── */

export type TagListParams = { sort: "alpha" | "count" };

export function parseTagListParams(url: URL): ParseResult<TagListParams> {
  const errors: ParseError[] = [];
  let sort: "alpha" | "count" = "count";
  const raw = url.searchParams.get("sort");
  if (raw !== null) {
    if (raw === "alpha" || raw === "count") sort = raw;
    else {
      errors.push({
        field: "sort",
        message: "Must be one of: alpha, count.",
      });
    }
  }
  if (errors.length > 0) return { ok: false, errors };
  return { ok: true, value: { sort } };
}
