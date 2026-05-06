/**
 * Public surface of the comment domain.
 *
 * Three surfaces import this barrel: the web server actions
 * (lib/actions/comment.ts), the REST endpoints (app/api/v1/comments/*),
 * and the MCP tools (lib/mcp/tools.ts). Auth happens at each
 * surface's boundary; the lifecycle verbs trust the authorId they
 * are given.
 *
 * Internal helpers (loadAuthorContext, the karma gate, the
 * confirmation-pass runner) stay private to this directory.
 */

export {
  commentInputSchema,
  updateCommentInputSchema,
} from "./schema";
export type {
  CommentInput,
  CommentResult,
  DeleteCommentResult,
  UpdateCommentInput,
  UpdateCommentResult,
} from "./schema";

export { createComment } from "./create";
export { updateCommentAsAuthor } from "./edit";
export { deleteCommentAsAuthor } from "./delete";
