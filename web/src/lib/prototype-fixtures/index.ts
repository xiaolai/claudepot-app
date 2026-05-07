/**
 * Public surface of the prototype-fixtures domain.
 *
 * Every reader-page that hits prototype JSON fixtures imports from
 * `@/lib/prototype-fixtures`. This barrel preserves that path while
 * the underlying code is split into per-domain files. The split is
 * cosmetic — fixture data, types, and queries are 1:1 with what the
 * monolith exposed.
 */

export type {
  CommentNode,
  Notification,
  PodcastMeta,
  Project,
  Submission,
  SubmissionType,
  Tag,
  ToolMeta,
  User,
} from "./types";

export {
  getAllCommentsForSubmission,
  getAllSubmissions,
  getAllUsers,
  getCommentsForSubmission,
  getPendingForUser,
  getSavedForUser,
  getSubmissionById,
  getSubmissionsByHot,
  getSubmissionsByNew,
  getSubmissionsByTop,
  getSubmissionsByUser,
  getUpvotedByUser,
  getUser,
  hotScore,
} from "./submissions";

export {
  getAllTags,
  getSubmissionsByTag,
  getTagBySlug,
  getTopTags,
} from "./tags";

export {
  getAllProjects,
  getProjectBySlug,
  getRelatedSubmissionsForProject,
} from "./projects";

export {
  getNotificationsForUser,
  unreadNotificationCount,
} from "./notifications";
