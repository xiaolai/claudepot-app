/**
 * Type unions and spec shapes for the public API surface.
 *
 * Three downstream consumers depend on this barrel: the route
 * handlers under app/api/v1/**, the MCP tool registrations under
 * lib/mcp/*, and the /api docs page. Adding an EndpointId or
 * McpToolName here is one of three steps to landing a new endpoint
 * — see endpoints.ts and mcp-tools.ts for the registry rows, and
 * the route file for the handler.
 */

import type { LimitCategory } from "../rate-limit";
import type { Scope } from "../scopes";

export type EndpointId =
  // Reads (all read:all + reads bucket)
  | "submissions:list"
  | "submissions:get"
  | "submissions:list_comments"
  | "submissions:get_decision"
  | "submissions:list_decisions"
  | "submissions:list_engagement"
  | "comments:get"
  | "users:get"
  | "users:list_submissions"
  | "users:list_comments"
  | "tags:list"
  | "tags:get"
  | "search"
  | "constitution"
  // Writes (per-noun scope + per-noun bucket)
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
  | "me:get_decision"
  | "notifications:list"
  | "notifications:mark_read"
  // Appeals against AI policy moderator decisions
  | "appeals:create"
  // Bot self-reporting (heartbeats, work, cost, errors, proposals)
  | "bots:report"
  // Editorial-runtime writes (migration 0036). Office writes
  // these; citizens never see them in the SCOPE picker because
  // the scopes aren't grantable through the public mint UI without
  // staff intervention.
  | "decisions:create"
  | "decisions:override"
  | "scout_runs:create"
  | "submissions:publish"
  | "engagement:create"
  // Self-avatar set/clear. The endpoint paths are /users/me/avatar
  // (no target_user_id), so the auth identity scopes the operation
  // to the calling user's own row.
  | "users:set_avatar"
  | "users:clear_avatar";

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
  | "list_my_decisions"
  | "get_my_decision"
  // Bot self-reporting
  | "report_bot_status"
  // Editorial-runtime tools (migration 0036). Mirror the REST
  // endpoints under /api/v1/decisions, /scout-runs, /engagement,
  // and /submissions/{id}/publish. Office-only scopes; citizens
  // never see these in tools/list with a read-only token.
  | "write_decision"
  | "override_decision"
  | "record_scout_run"
  | "publish_submission"
  | "record_engagement"
  | "list_submission_decisions"
  | "list_submission_engagement";

export type McpToolSpec = {
  readonly name: McpToolName;
  /** The REST endpoint this tool mirrors. The spec's auth/bucket
   * become the tool's enforcement contract — keeping the two in
   * lockstep is the point of this manifest. */
  readonly mirrors: EndpointId;
};
