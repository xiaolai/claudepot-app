/**
 * Type aliases for the prototype-fixtures domain.
 *
 * Mirrors the JSON fixture shape under design/fixtures/. Most fields
 * come from the v0 reader prototype (cookie-only, no DB); a few were
 * added when the app started reading from Neon and we needed prototype
 * fixtures to match real rows (e.g. updated_at after migration 0017).
 */

import type { AIDecision, ModerationState } from "@/lib/moderation-fixtures";

export type SubmissionType =
  | "news"
  | "tip"
  | "tutorial"
  | "course"
  | "article"
  | "podcast"
  | "interview"
  | "tool"
  | "discussion"
  // Added in 0008_editorial_runtime — match editorial/rubric.yml v0.2.3
  // submission types. Must stay in sync with src/db/schema.ts
  // submissionTypeEnum.
  | "release"
  | "paper"
  | "workflow"
  | "case_study"
  | "prompt_pattern";

export interface ToolMeta {
  stars: number;
  language: string;
  last_commit_relative: string;
}

export interface PodcastMeta {
  duration_min: number;
  host: string;
}

export interface Submission {
  id: string;
  user: string;
  user_image_url?: string | null;
  type: SubmissionType;
  /** Type as classified by the office's editorial mesh (the most-recent
   * decision_records.type_inferred for this submission). Set only when
   * the author is is_agent=true AND a decision exists; absent for
   * citizen submissions and for pre-decision bot submissions. The badge
   * renders this in preference to `type` so the office's correction of
   * a lazy initial classification is what readers see. */
  effective_type?: SubmissionType;
  tags: string[];
  title: string;
  url: string | null;
  domain: string;
  subjects: string[];
  upvotes: number;
  downvotes: number;
  comments: number;
  submitted_at: string;
  /** Set only by post-window edits (see migration 0017). UI shows
   * an "edited <relative>" badge iff this is present. Within-window
   * edits stay silent and leave this undefined. */
  updated_at?: string;
  text?: string;
  auto_posted?: boolean;
  reading_time_min?: number;
  tool_meta?: ToolMeta;
  podcast_meta?: PodcastMeta;
  state?: ModerationState;
  ai_decision?: AIDecision;
}

export interface CommentNode {
  id: string;
  user: string;
  user_image_url?: string | null;
  submitted_at: string;
  /** Same semantics as Submission.updated_at. */
  updated_at?: string;
  upvotes: number;
  downvotes: number;
  body: string;
  children: CommentNode[];
  state?: ModerationState;
  ai_decision?: AIDecision;
  /**
   * Set by the loader when this node represents a tombstone — either
   * its row has deletedAt set, or it was filtered out by the public-
   * visibility predicate but kept in the tree because descendants are
   * visible. Components key off this flag rather than checking
   * body === "[deleted]", which would collide with a legitimate
   * comment that happens to contain that literal text.
   */
  tombstoned?: boolean;
}

export interface User {
  username: string;
  display_name: string;
  karma: number;
  joined: string;
  bio: string;
  provider: string;
  is_system?: boolean;
  image_url?: string | null;
}

export interface Project {
  slug: string;
  name: string;
  tagline: string;
  repo_url: string;
  site_url: string | null;
  primary_language: string | null;
  stars: number;
  updated_at?: string;
  // Long-form fields, populated by the GitHub sync (`readme_md`) and
  // hand-authored editorial layer (`editorial_md`). Both nullable —
  // the detail page renders gracefully when either is absent.
  readme_md?: string | null;
  editorial_md?: string | null;
}

export interface Tag {
  slug: string;
  name: string;
  tagline: string;
}

export interface Notification {
  id: string;
  kind:
    | "reply"
    | "mention"
    | "milestone"
    | "system"
    | "mod-approved"
    | "mod-rejected";
  body: string;
  link: string;
  unread: boolean;
  at: string;
}

