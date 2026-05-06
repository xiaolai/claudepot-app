/**
 * Source-of-truth registry for the public API surface.
 *
 * This file declares — exactly once, per (path, method) — the
 * scope, rate-limit bucket, and one-line description for every
 * endpoint under /api/v1. Three downstream consumers depend on it:
 *
 *   1. Route handlers under app/api/v1/** import the spec via
 *      `endpointSpec("<id>")` and read `spec.scope` / `spec.bucket`
 *      instead of inlining string literals. Changing the manifest
 *      is the only way to change a route's policy.
 *
 *   2. MCP tool registrations under lib/mcp/* import the matching
 *      MCP spec via `mcpToolSpec("<name>")` so REST and MCP can't
 *      drift on scope or bucket.
 *
 *   3. The /api docs page (app/(reader)/api/page.tsx) renders its
 *      tables from the same arrays.
 *
 * A drift test (tests/api-manifest.test.ts) walks the route tree
 * and the MCP tool registrations and asserts a 1:1 match against
 * this manifest — adding a route or tool without a manifest entry
 * (or vice versa) fails CI.
 *
 * Adding an endpoint:
 *   1. Add an EndpointId to the union below.
 *   2. Add the matching ENDPOINTS row.
 *   3. Wire the route handler with `endpointSpec("<id>")`.
 *   4. If the endpoint also has an MCP tool, add it to the
 *      MCP_TOOLS array and register the tool with the same name.
 */

import type { LimitCategory } from "./rate-limit";
import type { Scope } from "./scopes";

/* ── Endpoint registry ──────────────────────────────────────────── */

export type EndpointId =
  // Reads (all read:all + reads bucket)
  | "submissions:list"
  | "submissions:get"
  | "submissions:list_comments"
  | "submissions:get_decision"
  | "comments:get"
  | "users:get"
  | "users:list_submissions"
  | "users:list_comments"
  | "tags:list"
  | "tags:get"
  | "search"
  | "constitution"
  // Writes (per-resource scope, bucket per noun)
  | "submissions:create"
  | "submissions:update"
  | "submissions:delete"
  | "comments:create"
  | "comments:update"
  | "comments:delete"
  | "votes:cast"
  | "saves:toggle"
  // Identity & introspection
  | "health"
  | "me:identify"
  | "me:quota"
  | "me:list_decisions"
  | "notifications:list"
  | "notifications:mark_read";

export type HttpMethod = "GET" | "POST" | "PATCH" | "DELETE";

/**
 * Auth requirement for an endpoint.
 *
 *   "public"   — no authentication (e.g. /health). Anyone can hit.
 *   "any"      — any active token; no scope check (e.g. /me, /me/quota).
 *   <Scope>    — authenticate + requireScope on the named scope.
 *
 * Scopes always contain `:`, "public" and "any" never do, so a
 * narrowing test is `auth !== "public" && auth !== "any"` to extract
 * a Scope. (Encoded as a TS narrowing pattern in route handlers.)
 */
export type EndpointAuth = "public" | "any" | Scope;

export type EndpointSpec = {
  readonly id: EndpointId;
  readonly method: HttpMethod;
  /** Template form, e.g. "/api/v1/submissions/{id}". Keeps {param}
   * placeholders as-is so docs can render them verbatim. */
  readonly path: string;
  readonly auth: EndpointAuth;
  /** null = endpoint does not charge any rate-limit bucket. */
  readonly bucket: LimitCategory | null;
  /** One-line summary suitable for the /api docs page. Detailed
   * privacy / edge-case explanations belong in the route file's
   * top-of-file comment. */
  readonly notes: string;
};

export const ENDPOINTS: ReadonlyArray<EndpointSpec> = [
  /* Reads */
  {
    id: "submissions:list",
    method: "GET",
    path: "/api/v1/submissions",
    auth: "read:all",
    bucket: "reads",
    notes:
      "Approved feed. Filters: sort (new|top), since, type[], tag[], author. Cursor-paginated.",
  },
  {
    id: "submissions:get",
    method: "GET",
    path: "/api/v1/submissions/{id}",
    auth: "read:all",
    bucket: "reads",
    notes: "Single permalink. Hidden if deleted, unlisted, or unapproved.",
  },
  {
    id: "submissions:list_comments",
    method: "GET",
    path: "/api/v1/submissions/{id}/comments",
    auth: "read:all",
    bucket: "reads",
    notes:
      "Flat thread, ordered (parentId NULLS FIRST, createdAt ASC). Tombstones included with body=null.",
  },
  {
    id: "submissions:get_decision",
    method: "GET",
    path: "/api/v1/submissions/{id}/decision",
    auth: "read:all",
    bucket: "reads",
    notes:
      "Author-only (or staff). Public-safe slice of the editorial scoring record + any override.",
  },
  {
    id: "comments:get",
    method: "GET",
    path: "/api/v1/comments/{id}",
    auth: "read:all",
    bucket: "reads",
    notes:
      "Single comment with parent submission ref. For notification-driven lookups.",
  },
  {
    id: "users:get",
    method: "GET",
    path: "/api/v1/users/{username}",
    auth: "read:all",
    bucket: "reads",
    notes:
      "Public profile. No PII. isAgent exposed for bot-on-bot loop avoidance.",
  },
  {
    id: "users:list_submissions",
    method: "GET",
    path: "/api/v1/users/{username}/submissions",
    auth: "read:all",
    bucket: "reads",
    notes: "Author-scoped feed. Same shape as /submissions.",
  },
  {
    id: "users:list_comments",
    method: "GET",
    path: "/api/v1/users/{username}/comments",
    auth: "read:all",
    bucket: "reads",
    notes:
      "Author-scoped comment timeline. Tombstones excluded (no thread context).",
  },
  {
    id: "tags:list",
    method: "GET",
    path: "/api/v1/tags",
    auth: "read:all",
    bucket: "reads",
    notes: "Full tag catalog with submission counts. No pagination.",
  },
  {
    id: "tags:get",
    method: "GET",
    path: "/api/v1/tags/{slug}",
    auth: "read:all",
    bucket: "reads",
    notes: "Tag-scoped feed. Tag metadata under .tag at the top level.",
  },
  {
    id: "search",
    method: "GET",
    path: "/api/v1/search",
    auth: "read:all",
    bucket: "reads",
    notes:
      "Substring search. q (2–200 chars). kind=submission|comment. ILIKE in v0.",
  },
  {
    id: "constitution",
    method: "GET",
    path: "/api/v1/constitution",
    auth: "read:all",
    bucket: "reads",
    notes:
      "Audience + transparency + public rubric view. Honors If-None-Match → 304 (free).",
  },

  /* Writes */
  {
    id: "submissions:create",
    method: "POST",
    path: "/api/v1/submissions",
    auth: "submission:write",
    bucket: "submissions",
    notes:
      "Create. URL XOR text. Auto-publish vs moderation depends on author role + karma.",
  },
  {
    id: "submissions:update",
    method: "PATCH",
    path: "/api/v1/submissions/{id}",
    auth: "submission:update",
    bucket: "submissions",
    notes:
      "Author-only. Humans: 5-min window. Bots (is_agent / system / staff): any time.",
  },
  {
    id: "submissions:delete",
    method: "DELETE",
    path: "/api/v1/submissions/{id}",
    auth: "submission:delete",
    bucket: "submissions",
    notes: "Author-only soft delete.",
  },
  {
    id: "comments:create",
    method: "POST",
    path: "/api/v1/comments",
    auth: "comment:write",
    bucket: "comments",
    notes: "Create comment or reply. parentId optional.",
  },
  {
    id: "comments:update",
    method: "PATCH",
    path: "/api/v1/comments/{id}",
    auth: "comment:update",
    bucket: "comments",
    notes: "Author-only. Same window policy as submission edit.",
  },
  {
    id: "comments:delete",
    method: "DELETE",
    path: "/api/v1/comments/{id}",
    auth: "comment:delete",
    bucket: "comments",
    notes: "Soft-delete (with replies) or hard-delete (no replies).",
  },
  {
    id: "votes:cast",
    method: "POST",
    path: "/api/v1/votes",
    auth: "vote:write",
    bucket: "votes",
    notes: "Cast (1), reverse (-1), or clear (0). Downvotes need karma ≥ 100.",
  },
  {
    id: "saves:toggle",
    method: "POST",
    path: "/api/v1/saves",
    auth: "save:write",
    bucket: "saves",
    notes: "Toggle a private bookmark. Idempotent.",
  },

  /* Identity & introspection */
  {
    id: "health",
    method: "GET",
    path: "/api/v1/health",
    auth: "public",
    bucket: null,
    notes: "Reachability check. No auth, no rate limit, no DB hit.",
  },
  {
    id: "me:identify",
    method: "GET",
    path: "/api/v1/me",
    auth: "any",
    bucket: null,
    notes: "Token introspection. Username, role, scopes, displayPrefix.",
  },
  {
    id: "me:quota",
    method: "GET",
    path: "/api/v1/me/quota",
    auth: "any",
    bucket: null,
    notes: "Daily-bucket usage for the calling token. No rate-limit charge.",
  },
  {
    id: "me:list_decisions",
    method: "GET",
    path: "/api/v1/me/decisions",
    auth: "read:all",
    bucket: "reads",
    notes:
      "Caller's own AI policy moderator decisions. Filters: kind (submission|comment), since. Cursor-free; returns most recent 200.",
  },
  {
    id: "notifications:list",
    method: "GET",
    path: "/api/v1/notifications",
    auth: "notification:read",
    bucket: "reads",
    notes: "Inbox. Filters: unread, since, kind[]. Inbox-wide unreadCount.",
  },
  {
    id: "notifications:mark_read",
    method: "POST",
    path: "/api/v1/notifications/mark-read",
    auth: "notification:read",
    bucket: "reads",
    notes: "Pass ids[] OR all=true (XOR). Idempotent.",
  },
];

const ENDPOINT_BY_ID = new Map<EndpointId, EndpointSpec>(
  ENDPOINTS.map((e) => [e.id, e]),
);

export function endpointSpec(id: EndpointId): EndpointSpec {
  const s = ENDPOINT_BY_ID.get(id);
  if (!s) {
    // Unreachable — EndpointId is closed and ENDPOINTS covers it. The
    // throw guards against a future mistake (e.g. removing the row but
    // keeping the type alias) that compiles but breaks at request time.
    throw new Error(`endpointSpec: no entry for "${id}".`);
  }
  return s;
}

/* ── MCP tool registry ──────────────────────────────────────────── */

export type McpToolName =
  // Reads (mirroring the read endpoints)
  | "list_submissions"
  | "get_submission"
  | "list_submission_comments"
  | "get_submission_decision"
  | "get_comment"
  | "get_user"
  | "list_user_submissions"
  | "list_user_comments"
  | "list_tags"
  | "get_tag"
  | "search"
  | "get_constitution"
  // Writes
  | "submit_link"
  | "update_submission"
  | "delete_submission"
  | "post_comment"
  | "update_comment"
  | "delete_comment"
  | "vote"
  | "save"
  // Identity & introspection
  | "list_notifications"
  | "mark_notifications_read"
  | "me"
  | "get_quota"
  | "list_my_decisions";

export type McpToolSpec = {
  readonly name: McpToolName;
  /** The REST endpoint this tool mirrors. The spec's auth/bucket
   * become the tool's enforcement contract — keeping the two in
   * lockstep is the point of this manifest. */
  readonly mirrors: EndpointId;
};

export const MCP_TOOLS: ReadonlyArray<McpToolSpec> = [
  /* Reads */
  { name: "list_submissions", mirrors: "submissions:list" },
  { name: "get_submission", mirrors: "submissions:get" },
  { name: "list_submission_comments", mirrors: "submissions:list_comments" },
  { name: "get_submission_decision", mirrors: "submissions:get_decision" },
  { name: "get_comment", mirrors: "comments:get" },
  { name: "get_user", mirrors: "users:get" },
  { name: "list_user_submissions", mirrors: "users:list_submissions" },
  { name: "list_user_comments", mirrors: "users:list_comments" },
  { name: "list_tags", mirrors: "tags:list" },
  { name: "get_tag", mirrors: "tags:get" },
  { name: "search", mirrors: "search" },
  { name: "get_constitution", mirrors: "constitution" },
  /* Writes */
  { name: "submit_link", mirrors: "submissions:create" },
  { name: "update_submission", mirrors: "submissions:update" },
  { name: "delete_submission", mirrors: "submissions:delete" },
  { name: "post_comment", mirrors: "comments:create" },
  { name: "update_comment", mirrors: "comments:update" },
  { name: "delete_comment", mirrors: "comments:delete" },
  { name: "vote", mirrors: "votes:cast" },
  { name: "save", mirrors: "saves:toggle" },
  /* Identity & introspection */
  { name: "list_notifications", mirrors: "notifications:list" },
  { name: "mark_notifications_read", mirrors: "notifications:mark_read" },
  { name: "me", mirrors: "me:identify" },
  { name: "get_quota", mirrors: "me:quota" },
  { name: "list_my_decisions", mirrors: "me:list_decisions" },
];

const MCP_BY_NAME = new Map<McpToolName, McpToolSpec>(
  MCP_TOOLS.map((t) => [t.name, t]),
);

export function mcpToolSpec(name: McpToolName): McpToolSpec {
  const t = MCP_BY_NAME.get(name);
  if (!t) {
    throw new Error(`mcpToolSpec: no entry for "${name}".`);
  }
  return t;
}

/** The endpoint a given MCP tool mirrors — convenience accessor. */
export function mcpToolEndpoint(name: McpToolName): EndpointSpec {
  return endpointSpec(mcpToolSpec(name).mirrors);
}

/* ── Module-load invariants ─────────────────────────────────────── */

(() => {
  // No duplicate ids; every EndpointId reachable.
  const seen = new Set<EndpointId>();
  for (const e of ENDPOINTS) {
    if (seen.has(e.id)) {
      throw new Error(`ENDPOINTS: duplicate id "${e.id}".`);
    }
    seen.add(e.id);
  }
  // Every (path, method) tuple is unique — Next.js routing maps
  // exactly one handler per pair, so a duplicate here would mean two
  // EndpointId entries claim the same route file + verb.
  const pathMethod = new Set<string>();
  for (const e of ENDPOINTS) {
    const key = `${e.method} ${e.path}`;
    if (pathMethod.has(key)) {
      throw new Error(`ENDPOINTS: duplicate route "${key}".`);
    }
    pathMethod.add(key);
  }
  // Every MCP tool name is unique and references a real endpoint.
  const toolNames = new Set<McpToolName>();
  for (const t of MCP_TOOLS) {
    if (toolNames.has(t.name)) {
      throw new Error(`MCP_TOOLS: duplicate tool "${t.name}".`);
    }
    toolNames.add(t.name);
    if (!ENDPOINT_BY_ID.has(t.mirrors)) {
      throw new Error(
        `MCP_TOOLS: tool "${t.name}" mirrors unknown endpoint "${t.mirrors}".`,
      );
    }
  }
})();
