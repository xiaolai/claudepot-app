/**
 * Endpoint registry — one row per (path, method) under /api/v1.
 *
 * Adding an endpoint:
 *   1. Add the EndpointId to manifest/types.ts.
 *   2. Add the matching ENDPOINTS row here.
 *   3. Wire the route handler with `endpointSpec("<id>")`.
 *
 * The drift test (tests/api-manifest.test.ts) walks the route tree
 * and asserts a 1:1 match — adding a route without a manifest entry
 * (or vice versa) fails CI.
 */

import type { EndpointId, EndpointSpec } from "./types";

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
    id: "me:get_decision",
    method: "GET",
    path: "/api/v1/me/decisions/{id}",
    auth: "read:all",
    bucket: "reads",
    notes:
      "Caller's own AI policy moderator decision by id. Cross-user access returns 404 (not 403) so existence isn't disclosed.",
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
  {
    id: "appeals:create",
    method: "POST",
    path: "/api/v1/appeals",
    auth: "any",
    bucket: null,
    notes:
      "Appeal a policy_decisions reject. Body: { decisionId, text }. Caller must own the decision. One open appeal per target — duplicates rejected.",
  },
];

export const ENDPOINT_BY_ID: ReadonlyMap<EndpointId, EndpointSpec> = new Map(
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
