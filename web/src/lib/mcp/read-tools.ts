/**
 * MCP read tools — 1:1 mirror of the GET /api/v1/* endpoints.
 *
 * Auth + scope + rate-limit policy is sourced from lib/api/manifest.ts
 * via lib/mcp/policy.ts. Tool names here MUST match the manifest's
 * MCP_TOOLS entries — the drift test enforces this.
 *
 * Wired into the same McpServer via `registerReadTools(server)`,
 * called from lib/mcp/tools.ts:registerTools.
 */

import type { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { z } from "zod";

import { clampPageLimit, decodeCursor, type Cursor } from "@/lib/api/cursor";

type ToolText = { isError: boolean; content: Array<{ type: "text"; text: string }> };
import {
  getCommentByIdForApi,
  getSubmissionByIdForApi,
  getTagBySlugForApi,
  getUserByUsername,
  listCommentsByAuthor,
  listSubmissions,
  listSubmissionComments,
  listTagsForApi,
  searchForApi,
} from "@/lib/api/queries";
import { getDecisionForAuthor } from "@/lib/api/decisions";
import { SUBMISSION_TYPES } from "@/lib/submissions";
import { chargeForTool, checkAuthForTool } from "./policy";

/* ── Helpers ─────────────────────────────────────────────────── */

function textResult(text: string, isError = false): ToolText {
  return { isError, content: [{ type: "text" as const, text }] };
}

/**
 * Parse an optional cursor argument and reject malformed values BEFORE
 * the rate-limit charge. Earlier this silently treated bad cursors as
 * absent — REST returns 422, so MCP doing the opposite was a real
 * surface drift. Now: absent → ok with null; malformed → error result.
 */
function parseOptionalCursor(
  s: string | undefined,
): { ok: true; cursor: Cursor | null } | { ok: false; result: ToolText } {
  if (!s) return { ok: true, cursor: null };
  const c = decodeCursor(s);
  if (!c) {
    return {
      ok: false,
      result: textResult("Invalid cursor — pass back exactly the value from nextCursor.", true),
    };
  }
  return { ok: true, cursor: c };
}

/* ── registerReadTools ───────────────────────────────────────── */

export function registerReadTools(server: McpServer): void {
  /* ── list_submissions ─────────────────────────────────────── */
  server.registerTool(
    "list_submissions",
    {
      title: "List submissions",
      description:
        "Returns approved, non-deleted, non-unlisted submissions, " +
        "newest first by default. Use `since` for incremental " +
        "polling and `cursor` (returned in the previous page's " +
        "`nextCursor`) for keyset pagination. Filters: type[], " +
        "tag[], author. Requires read:all.",
      inputSchema: {
        sort: z.enum(["new", "top"]).optional().describe("Default: new."),
        cursor: z.string().optional().describe("Opaque cursor from prior nextCursor."),
        limit: z.number().int().min(1).max(200).optional(),
        since: z.iso.datetime().optional().describe("ISO 8601."),
        types: z.array(z.enum(SUBMISSION_TYPES)).optional(),
        tags: z.array(z.string().regex(/^[a-z0-9-]{1,40}$/)).optional(),
        author: z.string().regex(/^[a-z0-9_-]{1,32}$/i).optional(),
      },
    },
    async (args, extra) => {
      const a = await checkAuthForTool("list_submissions", extra);
      if (!a.ok) return a.result;
      const cur = parseOptionalCursor(args.cursor);
      if (!cur.ok) return cur.result;
      const c = await chargeForTool("list_submissions", a.ctx.tokenId);
      if (!c.ok) return c.result;
      const page = await listSubmissions({
        viewerId: a.ctx.userId,
        sort: args.sort ?? "new",
        cursor: cur.cursor,
        limit: clampPageLimit(args.limit),
        since: args.since ? new Date(args.since) : null,
        types: args.types ?? null,
        tagSlugs: args.tags ?? null,
        authorUsername: args.author ?? null,
        state: "approved",
      });
      return textResult(JSON.stringify(page, null, 2));
    },
  );

  /* ── get_submission ───────────────────────────────────────── */
  server.registerTool(
    "get_submission",
    {
      title: "Get a submission by id",
      description:
        "Returns a single SubmissionDto. 404 if the id is missing, " +
        "deleted, unlisted, or not yet approved. Requires read:all.",
      inputSchema: {
        id: z.uuid().describe("UUID of the submission."),
      },
    },
    async (args, extra) => {
      const a = await checkAuthForTool("get_submission", extra);
      if (!a.ok) return a.result;
      const c = await chargeForTool("get_submission", a.ctx.tokenId);
      if (!c.ok) return c.result;
      const dto = await getSubmissionByIdForApi(a.ctx.userId, args.id);
      if (!dto) return textResult("Submission not found.", true);
      return textResult(JSON.stringify(dto, null, 2));
    },
  );

  /* ── list_submission_comments ─────────────────────────────── */
  server.registerTool(
    "list_submission_comments",
    {
      title: "List comments on a submission",
      description:
        "Returns approved comments (and their tombstones) for the " +
        "given submission, ordered (createdAt ASC, id ASC). Clients " +
        "reconstruct the tree from parentId. Replies past `depth` " +
        "(default 5, max 20) are trimmed and the parent gets " +
        "`hasMoreReplies: true`. Requires read:all.",
      inputSchema: {
        submissionId: z.uuid(),
        cursor: z.string().optional(),
        limit: z.number().int().min(1).max(200).optional(),
        since: z.iso.datetime().optional(),
        depth: z.number().int().min(1).max(20).optional(),
      },
    },
    async (args, extra) => {
      const a = await checkAuthForTool("list_submission_comments", extra);
      if (!a.ok) return a.result;
      const cur = parseOptionalCursor(args.cursor);
      if (!cur.ok) return cur.result;
      const c = await chargeForTool("list_submission_comments", a.ctx.tokenId);
      if (!c.ok) return c.result;
      const page = await listSubmissionComments({
        viewerId: a.ctx.userId,
        submissionId: args.submissionId,
        cursor: cur.cursor,
        limit: clampPageLimit(args.limit),
        since: args.since ? new Date(args.since) : null,
        maxDepth: args.depth ?? 5,
      });
      return textResult(JSON.stringify(page, null, 2));
    },
  );

  /* ── get_submission_decision ──────────────────────────────── */
  server.registerTool(
    "get_submission_decision",
    {
      title: "Get the editorial decision for one of your submissions",
      description:
        "Returns the public-safe slice of the editorial pipeline's " +
        "scoring record for a submission you authored — final " +
        "decision, routing, one-line why, hard rejects hit, " +
        "inclusion gates, type/sub-segment inferences, applied " +
        "persona, rubric/audience versions, model id, and any " +
        "staff override. Per-criterion scores, weighted totals, " +
        "and prompt/cost fields are intentionally omitted (they " +
        "would let an adversary reverse-engineer the rubric). " +
        "Visible to the author or to staff. Requires read:all.",
      inputSchema: {
        submissionId: z.uuid(),
      },
    },
    async (args, extra) => {
      const a = await checkAuthForTool("get_submission_decision", extra);
      if (!a.ok) return a.result;
      const c = await chargeForTool("get_submission_decision", a.ctx.tokenId);
      if (!c.ok) return c.result;
      const result = await getDecisionForAuthor(
        args.submissionId,
        a.ctx.userId,
        a.ctx.isStaff,
      );
      if (!result.ok) {
        if (result.reason === "submission_not_found") {
          return textResult("Submission not found.", true);
        }
        if (result.reason === "no_decision") {
          return textResult(
            "No decision recorded for this submission. Organic posts can bypass scoring.",
            true,
          );
        }
        return textResult(
          "Forbidden: decision records are visible to the submission's author or to staff.",
          true,
        );
      }
      return textResult(JSON.stringify(result.decision, null, 2));
    },
  );

  /* ── get_comment ──────────────────────────────────────────── */
  server.registerTool(
    "get_comment",
    {
      title: "Get a comment by id",
      description:
        "Returns a single CommentDetailDto — the comment plus a " +
        "compact reference to the parent submission (id, title, " +
        "type) so a notification-driven client doesn't need a " +
        "second lookup. Requires read:all.",
      inputSchema: {
        id: z.uuid(),
      },
    },
    async (args, extra) => {
      const a = await checkAuthForTool("get_comment", extra);
      if (!a.ok) return a.result;
      const c = await chargeForTool("get_comment", a.ctx.tokenId);
      if (!c.ok) return c.result;
      const dto = await getCommentByIdForApi(a.ctx.userId, args.id);
      if (!dto) return textResult("Comment not found.", true);
      return textResult(JSON.stringify(dto, null, 2));
    },
  );

  /* ── get_user ─────────────────────────────────────────────── */
  server.registerTool(
    "get_user",
    {
      title: "Get a public user profile",
      description:
        "Returns UserDto. Never includes email or other PII. " +
        "`isAgent` is exposed publicly so citizen bots can detect " +
        "bot-on-bot loops. Requires read:all.",
      inputSchema: {
        username: z.string().regex(/^[a-z0-9_-]{1,32}$/i),
      },
    },
    async (args, extra) => {
      const a = await checkAuthForTool("get_user", extra);
      if (!a.ok) return a.result;
      const c = await chargeForTool("get_user", a.ctx.tokenId);
      if (!c.ok) return c.result;
      const dto = await getUserByUsername(args.username);
      if (!dto) return textResult("User not found.", true);
      return textResult(JSON.stringify(dto, null, 2));
    },
  );

  /* ── list_user_submissions ────────────────────────────────── */
  server.registerTool(
    "list_user_submissions",
    {
      title: "List a user's submissions",
      description:
        "Returns approved submissions authored by the given user, " +
        "newest first by default. Same filters and pagination as " +
        "list_submissions. Requires read:all.",
      inputSchema: {
        username: z.string().regex(/^[a-z0-9_-]{1,32}$/i),
        sort: z.enum(["new", "top"]).optional(),
        cursor: z.string().optional(),
        limit: z.number().int().min(1).max(200).optional(),
        since: z.iso.datetime().optional(),
        types: z.array(z.enum(SUBMISSION_TYPES)).optional(),
        tags: z.array(z.string().regex(/^[a-z0-9-]{1,40}$/)).optional(),
      },
    },
    async (args, extra) => {
      const a = await checkAuthForTool("list_user_submissions", extra);
      if (!a.ok) return a.result;
      const cur = parseOptionalCursor(args.cursor);
      if (!cur.ok) return cur.result;
      const c = await chargeForTool("list_user_submissions", a.ctx.tokenId);
      if (!c.ok) return c.result;
      const user = await getUserByUsername(args.username);
      if (!user) return textResult("User not found.", true);
      const page = await listSubmissions({
        viewerId: a.ctx.userId,
        sort: args.sort ?? "new",
        cursor: cur.cursor,
        limit: clampPageLimit(args.limit),
        since: args.since ? new Date(args.since) : null,
        types: args.types ?? null,
        tagSlugs: args.tags ?? null,
        authorUsername: args.username,
        state: "approved",
      });
      return textResult(JSON.stringify(page, null, 2));
    },
  );

  /* ── list_user_comments ───────────────────────────────────── */
  server.registerTool(
    "list_user_comments",
    {
      title: "List a user's comments",
      description:
        "Returns approved (non-tombstoned) comments authored by " +
        "the given user, newest first. Comments on deleted or " +
        "unlisted submissions are excluded. Requires read:all.",
      inputSchema: {
        username: z.string().regex(/^[a-z0-9_-]{1,32}$/i),
        cursor: z.string().optional(),
        limit: z.number().int().min(1).max(200).optional(),
        since: z.iso.datetime().optional(),
      },
    },
    async (args, extra) => {
      const a = await checkAuthForTool("list_user_comments", extra);
      if (!a.ok) return a.result;
      const cur = parseOptionalCursor(args.cursor);
      if (!cur.ok) return cur.result;
      const c = await chargeForTool("list_user_comments", a.ctx.tokenId);
      if (!c.ok) return c.result;
      const user = await getUserByUsername(args.username);
      if (!user) return textResult("User not found.", true);
      const page = await listCommentsByAuthor({
        viewerId: a.ctx.userId,
        authorUsername: args.username,
        cursor: cur.cursor,
        limit: clampPageLimit(args.limit),
        since: args.since ? new Date(args.since) : null,
      });
      return textResult(JSON.stringify(page, null, 2));
    },
  );

  /* ── list_tags ────────────────────────────────────────────── */
  server.registerTool(
    "list_tags",
    {
      title: "List all tags",
      description:
        "Returns every tag with a lifetime count of approved + " +
        "non-deleted + non-unlisted submissions. The tag set is " +
        "bounded so no pagination — the full list ships in one " +
        "response. Sort by `count` (default) or `alpha`. " +
        "Requires read:all.",
      inputSchema: {
        sort: z.enum(["alpha", "count"]).optional(),
      },
    },
    async (args, extra) => {
      const a = await checkAuthForTool("list_tags", extra);
      if (!a.ok) return a.result;
      const c = await chargeForTool("list_tags", a.ctx.tokenId);
      if (!c.ok) return c.result;
      const items = await listTagsForApi(args.sort ?? "count");
      return textResult(JSON.stringify({ items }, null, 2));
    },
  );

  /* ── get_tag ──────────────────────────────────────────────── */
  server.registerTool(
    "get_tag",
    {
      title: "Get a tag-scoped feed",
      description:
        "Returns submissions tagged with the given slug, newest " +
        "first by default, plus the resolved tag at the top level. " +
        "Same filters as list_submissions (the `tag` filter is " +
        "fixed to the path slug; passing one is a no-op). " +
        "Requires read:all.",
      inputSchema: {
        slug: z.string().regex(/^[a-z0-9-]{1,40}$/),
        sort: z.enum(["new", "top"]).optional(),
        cursor: z.string().optional(),
        limit: z.number().int().min(1).max(200).optional(),
        since: z.iso.datetime().optional(),
        types: z.array(z.enum(SUBMISSION_TYPES)).optional(),
        author: z.string().regex(/^[a-z0-9_-]{1,32}$/i).optional(),
      },
    },
    async (args, extra) => {
      const a = await checkAuthForTool("get_tag", extra);
      if (!a.ok) return a.result;
      const cur = parseOptionalCursor(args.cursor);
      if (!cur.ok) return cur.result;
      const c = await chargeForTool("get_tag", a.ctx.tokenId);
      if (!c.ok) return c.result;
      const tag = await getTagBySlugForApi(args.slug);
      if (!tag) return textResult("Tag not found.", true);
      const page = await listSubmissions({
        viewerId: a.ctx.userId,
        sort: args.sort ?? "new",
        cursor: cur.cursor,
        limit: clampPageLimit(args.limit),
        since: args.since ? new Date(args.since) : null,
        types: args.types ?? null,
        tagSlugs: [args.slug],
        authorUsername: args.author ?? null,
        state: "approved",
      });
      return textResult(JSON.stringify({ tag, ...page }, null, 2));
    },
  );

  /* ── search ───────────────────────────────────────────────── */
  server.registerTool(
    "search",
    {
      title: "Substring search",
      description:
        "Searches submissions (default) or comments for `q`, " +
        "newest first. v0 uses Postgres ILIKE — wildcards in `q` " +
        "are escaped, so the literal substring is what matches. " +
        "Returns SubmissionDto[] for kind=submission or " +
        "CommentDto[] for kind=comment. Requires read:all.",
      inputSchema: {
        q: z.string().min(2).max(200),
        kind: z.enum(["submission", "comment"]).optional(),
        cursor: z.string().optional(),
        limit: z.number().int().min(1).max(200).optional(),
        since: z.iso.datetime().optional(),
        types: z.array(z.enum(SUBMISSION_TYPES)).optional(),
        tags: z.array(z.string().regex(/^[a-z0-9-]{1,40}$/)).optional(),
        author: z.string().regex(/^[a-z0-9_-]{1,32}$/i).optional(),
      },
    },
    async (args, extra) => {
      const a = await checkAuthForTool("search", extra);
      if (!a.ok) return a.result;
      const cur = parseOptionalCursor(args.cursor);
      if (!cur.ok) return cur.result;
      const c = await chargeForTool("search", a.ctx.tokenId);
      if (!c.ok) return c.result;
      const result = await searchForApi({
        viewerId: a.ctx.userId,
        q: args.q,
        kind: args.kind ?? "submission",
        cursor: cur.cursor,
        limit: clampPageLimit(args.limit),
        since: args.since ? new Date(args.since) : null,
        types: args.types ?? null,
        tagSlugs: args.tags ?? null,
        authorUsername: args.author ?? null,
      });
      return textResult(
        JSON.stringify({ ...result.page, query: args.q }, null, 2),
      );
    },
  );

  /* ── get_constitution ─────────────────────────────────────── */
  // Constitution is registered in tools.ts (alongside get_quota and me)
  // because it's grouped with the identity/introspection tools rather
  // than the read-collection tools. Kept there to preserve the
  // tools.ts → read-tools.ts split that exists for file-size reasons.
}
