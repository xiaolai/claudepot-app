/**
 * Public surface of the submission domain.
 *
 * Lives in lib/ (NOT lib/actions/) because three surfaces need it:
 *
 *   - Web UI server action (lib/actions/submission.ts:submitPost)
 *   - REST endpoint (app/api/v1/submissions/route.ts)
 *   - MCP tool (lib/mcp/tools.ts:submit_link)
 *
 * Auth happens at each surface's boundary; the create/edit/delete
 * functions trust the authorId they're given.
 *
 * Internal helpers (loadAuthorContext, findRecentDuplicate, the karma
 * gate) stay private to this directory — re-export only what callers
 * need so the import surface tracks behavior, not implementation.
 */

export {
  SUBMISSION_TYPES,
  submissionInputSchema,
  updateSubmissionInputSchema,
} from "./schema";
export type {
  SubmissionInput,
  SubmissionVia,
  SubmitResult,
  DeleteSubmissionResult,
  UpdateSubmissionInput,
  UpdateSubmissionResult,
} from "./schema";

export { createSubmission } from "./create";
export { updateSubmissionAsAuthor } from "./edit";
export { deleteSubmissionAsAuthor } from "./delete";
