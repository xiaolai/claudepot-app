/**
 * Postgres enums + the citext custom type. Kept together because
 * sibling schema files import them without their own internal
 * dependency tree.
 */

import { customType, pgEnum } from "drizzle-orm/pg-core";

/**
 * Postgres `citext` is case-insensitive text. Used for usernames
 * and emails so "Ada" and "ada" collide on the unique index.
 * The `citext` extension must be enabled at DB-init time
 * (see migrations/0001_enable_citext.sql).
 */
export const citext = customType<{ data: string; driverData: string }>({
  dataType: () => "citext",
});

export const userRoleEnum = pgEnum("user_role", [
  "user",
  "staff",
  "locked",
  "system",
]);

export const submissionTypeEnum = pgEnum("submission_type", [
  "news",
  "tip",
  "tutorial",
  "course",
  "article",
  "podcast",
  "interview",
  "tool",
  "discussion",
  // Added in 0008_editorial_runtime — match editorial/rubric.yml v0.2.3 types.
  // The five v1 types above (tip, course, article) stay for backward compat
  // with seeded fixture data; new scout submissions use the v0.2.3 types only.
  "release",
  "paper",
  "workflow",
  "case_study",
  "prompt_pattern",
]);

// Editorial runtime enums (added in 0008_editorial_runtime).
export const submitterKindEnum = pgEnum("submitter_kind", [
  "user",
  "scout",
  "import",
]);

export const aiFinalDecisionEnum = pgEnum("ai_final_decision", [
  "accept",
  "reject",
  "borderline_to_human_queue",
]);

export const routingDestinationEnum = pgEnum("routing_destination", [
  "feed",
  "firehose",
  "human_queue",
]);

export const confidenceBandEnum = pgEnum("confidence_band", [
  "high",
  "low",
]);

export const contentStateEnum = pgEnum("content_state", [
  "pending",
  "approved",
  "rejected",
]);

export const flagStatusEnum = pgEnum("flag_status", ["open", "resolved"]);

export const notificationKindEnum = pgEnum("notification_kind", [
  "comment_reply",
  "submission_reply",
  "moderation",
  "mention",
]);

export const moderationActionEnum = pgEnum("moderation_action", [
  "lock",
  "unlist",
  "delete",
  "restore",
  "dismiss_flag",
  "lock_user",
  "approve",
  "reject",
  "delete_hard",
  // Tag-vocabulary governance — added in migration 0011 so /admin/flags
  // CRUD shows up alongside content moderation in /admin/log.
  "tag_create",
  "tag_rename",
  "tag_merge",
  "tag_retire",
  // Bot-exempt toggle on /admin/users — added in migration 0019 so
  // grants and revokes can be filtered without parsing the note.
  "bot_exempt_grant",
  "bot_exempt_revoke",
]);

export const targetTypeEnum = pgEnum("target_type", [
  "submission",
  "comment",
  // User-level targets — added in migration 0019 so the policy
  // moderator's ban-candidate flags can point at the user under
  // review instead of a representative submission.
  "user",
]);

export const apiTokenEventEnum = pgEnum("api_token_event", [
  "mint",
  "revoke",
  "scope_change",
]);
