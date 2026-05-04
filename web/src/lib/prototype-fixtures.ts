import submissionsRaw from "../../design/fixtures/submissions.json";
import commentsRaw from "../../design/fixtures/comments.json";
import usersRaw from "../../design/fixtures/users.json";
import projectsRaw from "../../design/fixtures/projects.json";
import tagsRaw from "../../design/fixtures/tags.json";
import { ageHours } from "@/lib/format";
import {
  effectiveState,
  commentEffectiveState,
  type AIDecision,
  type ModerationState,
} from "@/lib/moderation";

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
  tags: string[];
  title: string;
  url: string | null;
  domain: string;
  subjects: string[];
  upvotes: number;
  downvotes: number;
  comments: number;
  submitted_at: string;
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

const submissions = submissionsRaw as Submission[];
const comments = commentsRaw as Record<string, CommentNode[]>;
const users = usersRaw as User[];
const projects = projectsRaw as Project[];
const tags = tagsRaw as Tag[];

// `ageHours` is computed relative to real-time `Date.now()`. The prototype
// previously hardcoded NOW = "2026-04-29T15:00:00Z" so fixture timestamps
// read as "1h ago"; now that the app reads from the live DB (real
// timestamps from gh CLI, real user submissions), Date.now() is correct.
// Fixtures rendered "fresh" during prototype iteration are now correctly
// older. The helper itself lives in @/lib/format.

/** Public-visible submissions: approved only. */
function publicVisible(): Submission[] {
  return submissions.filter((s) => effectiveState(s) === "approved");
}

/* ── Hot ranking ───────────────────────────────────────────── */

export function hotScore(s: Submission): number {
  const net = s.upvotes - s.downvotes;
  return Math.max(net - 1, 0) / Math.pow(ageHours(s.submitted_at) + 2, 1.8);
}

/* ── Submissions ───────────────────────────────────────────── */

export function getAllSubmissions(): Submission[] {
  return submissions;
}

export function getSubmissionsByHot(): Submission[] {
  return [...publicVisible()].sort((a, b) => hotScore(b) - hotScore(a));
}

export function getSubmissionsByNew(): Submission[] {
  return [...publicVisible()].sort(
    (a, b) =>
      new Date(b.submitted_at).getTime() - new Date(a.submitted_at).getTime(),
  );
}

export function getSubmissionsByTop(
  range: "day" | "week" | "all" = "day",
): Submission[] {
  const now = Date.now();
  const cutoff =
    range === "day"
      ? now - 86_400_000
      : range === "week"
        ? now - 7 * 86_400_000
        : 0;
  return publicVisible()
    .filter((s) => new Date(s.submitted_at).getTime() >= cutoff)
    .sort((a, b) => b.upvotes - b.downvotes - (a.upvotes - a.downvotes));
}

export function getSubmissionById(id: string): Submission | undefined {
  return submissions.find((s) => s.id === id);
}

/** Comments visible to public — approved only. */
export function getCommentsForSubmission(id: string): CommentNode[] {
  return filterCommentTree(comments[id] ?? [], (c) => commentEffectiveState(c) === "approved");
}

/** All comments including pending/rejected — used by the author and staff views. */
export function getAllCommentsForSubmission(id: string): CommentNode[] {
  return comments[id] ?? [];
}

function filterCommentTree(
  nodes: CommentNode[],
  predicate: (c: CommentNode) => boolean,
): CommentNode[] {
  return nodes
    .filter(predicate)
    .map((n) => ({ ...n, children: filterCommentTree(n.children, predicate) }));
}

export function getUser(username: string): User | undefined {
  return users.find((u) => u.username === username);
}

export function getAllUsers(): User[] {
  return users;
}

/** Submissions by a user — public-visible by default; pass `includeAll` for the author's own view. */
export function getSubmissionsByUser(
  username: string,
  includeAll = false,
): Submission[] {
  const pool = includeAll ? submissions : publicVisible();
  return pool
    .filter((s) => s.user === username)
    .sort(
      (a, b) =>
        new Date(b.submitted_at).getTime() -
        new Date(a.submitted_at).getTime(),
    );
}

/** Pending or rejected submissions for a user — author's own pending/queue view. */
export function getPendingForUser(username: string): Submission[] {
  return submissions
    .filter((s) => s.user === username && effectiveState(s) !== "approved")
    .sort(
      (a, b) =>
        new Date(b.submitted_at).getTime() -
        new Date(a.submitted_at).getTime(),
    );
}

/**
 * Saved (★ private bookmark) — distinct from upvotes (▲ public signal).
 * Stub: deterministic per-user list keyed off username.
 */
export function getSavedForUser(username: string): Submission[] {
  const seed = username.charCodeAt(0);
  const order = [...publicVisible()].sort((a, b) => b.upvotes - a.upvotes);
  return order.slice(seed % 4, (seed % 4) + 6);
}

/** Submissions a user has upvoted (separate from saved). Stub. */
export function getUpvotedByUser(username: string): Submission[] {
  const seed = username.charCodeAt(0) + 1;
  return [...publicVisible()]
    .sort((a, b) => hotScore(b) - hotScore(a))
    .slice(seed % 5, (seed % 5) + 8);
}

/* ── Tags ──────────────────────────────────────────────────── */

export function getAllTags(): Tag[] {
  return tags;
}

export function getTagBySlug(slug: string): Tag | undefined {
  return tags.find((t) => t.slug === slug);
}

/** Tags ranked by recent activity (count of approved submissions in last 7 days). */
export function getTopTags(): Array<Tag & { count: number }> {
  const cutoff = Date.now() - 7 * 86_400_000;
  const recent = publicVisible().filter(
    (s) => new Date(s.submitted_at).getTime() >= cutoff,
  );
  return tags
    .map((t) => ({
      ...t,
      count: recent.filter((s) => s.tags.includes(t.slug)).length,
    }))
    .sort((a, b) => b.count - a.count);
}

export function getSubmissionsByTag(slug: string): Submission[] {
  return publicVisible()
    .filter((s) => s.tags.includes(slug))
    .sort((a, b) => hotScore(b) - hotScore(a));
}

/* ── Projects ──────────────────────────────────────────────── */

export function getAllProjects(): Project[] {
  return projects;
}

export function getProjectBySlug(slug: string): Project | undefined {
  return projects.find((p) => p.slug === slug);
}

export function getRelatedSubmissionsForProject(
  slug: string,
  limit = 4,
): Submission[] {
  const project = getProjectBySlug(slug);
  if (!project) return [];
  const tokens = [project.slug, project.name.toLowerCase()];
  return publicVisible()
    .filter((s) =>
      tokens.some(
        (t) =>
          s.title.toLowerCase().includes(t) ||
          s.url?.toLowerCase().includes(t) ||
          false,
      ),
    )
    .slice(0, limit);
}

/* ── Notifications (mock) ──────────────────────────────────── */

export interface Notification {
  id: string;
  kind: "reply" | "mention" | "milestone" | "system" | "mod-approved" | "mod-rejected";
  body: string;
  link: string;
  unread: boolean;
  at: string;
}

export function getNotificationsForUser(username: string): Notification[] {
  const base: Notification[] = [
    {
      id: "n1",
      kind: "reply",
      body: "kai replied to your comment on Opus 4.7",
      link: "/post/1",
      unread: true,
      at: "2026-04-29T11:30:00Z",
    },
    {
      id: "n2",
      kind: "mention",
      body: "miro mentioned you on \"What's missing from Claude Code\"",
      link: "/post/36",
      unread: true,
      at: "2026-04-29T08:00:00Z",
    },
    {
      id: "n3",
      kind: "milestone",
      body: "Your post \"Building an eval harness\" passed 250 upvotes",
      link: "/post/2",
      unread: true,
      at: "2026-04-28T20:14:00Z",
    },
    {
      id: "n4",
      kind: "reply",
      body: "lin replied on \"one big agent or many small\"",
      link: "/post/8",
      unread: false,
      at: "2026-04-28T14:55:00Z",
    },
    {
      id: "n5",
      kind: "system",
      body: "Welcome — you have 3 unread notifications",
      link: "#",
      unread: false,
      at: "2026-04-28T09:00:00Z",
    },
  ];
  // Different users see different notifications — vary deterministically.
  const offset = username.charCodeAt(0) % 3;
  return base.slice(offset).map((n, i) => ({ ...n, id: `${username}-${i}` }));
}

export function unreadNotificationCount(username: string): number {
  return getNotificationsForUser(username).filter((n) => n.unread).length;
}

/* ── AI moderation queue (human review of borderline AI decisions) ── */

export interface ModQueueItem {
  id: string;
  target_type: "submission" | "comment";
  target_id: string;
  trigger: "low-confidence" | "user-flag" | "appeal";
  ai_confidence: number;
  ai_proposed_action: "approve" | "reject";
  ai_reason: string;
  flagged_by?: string;
  at: string;
}

/**
 * Human review queue — populated when AI confidence is low or a user flags
 * an already-published item, or when the author appeals a rejection.
 */
export function getModQueue(): ModQueueItem[] {
  return [
    {
      id: "q1",
      target_type: "submission",
      target_id: "p-pending-1",
      trigger: "low-confidence",
      ai_confidence: 0.62,
      ai_proposed_action: "approve",
      ai_reason: "Borderline self-promotion. Not a clear policy violation but the user has 3 prior posts to the same domain in the last week.",
      at: "2026-04-29T13:48:00Z",
    },
    {
      id: "q2",
      target_type: "submission",
      target_id: "p-rejected-1",
      trigger: "appeal",
      ai_confidence: 0.91,
      ai_proposed_action: "reject",
      ai_reason: "Affiliate link in body without disclosure. Author appealed, claims it's a personal-use tracker, not affiliate.",
      flagged_by: "ren",
      at: "2026-04-29T12:15:00Z",
    },
    {
      id: "q3",
      target_type: "comment",
      target_id: "c5-3",
      trigger: "user-flag",
      ai_confidence: 0.78,
      ai_proposed_action: "approve",
      ai_reason: "Heated but on-topic. User-flagged as personal attack; AI reads it as direct critique without ad hominem.",
      flagged_by: "ada",
      at: "2026-04-28T11:00:00Z",
    },
    {
      id: "q4",
      target_type: "submission",
      target_id: "p-pending-2",
      trigger: "low-confidence",
      ai_confidence: 0.71,
      ai_proposed_action: "approve",
      ai_reason: "Topic relevance uncertain — post is about general LLM ops, not Claude-specific. Could go either way.",
      at: "2026-04-29T14:22:00Z",
    },
  ];
}

/* ── AI audit log (every AI decision, for /admin/audit) ────── */

export interface AuditEntry {
  id: string;
  target_type: "submission" | "comment";
  target_id: string;
  action: "approve" | "reject" | "tag";
  reason: string;
  confidence: number;
  decided_at: string;
  overridden?: { by: string; new_action: "approve" | "reject"; at: string; note?: string };
}

export function getAuditLog(): AuditEntry[] {
  return [
    {
      id: "a1",
      target_type: "submission",
      target_id: "1",
      action: "approve",
      reason: "Official Anthropic news; tagged release-watch + long-context.",
      confidence: 0.99,
      decided_at: "2026-04-28T09:14:30Z",
    },
    {
      id: "a2",
      target_type: "submission",
      target_id: "p-rejected-1",
      action: "reject",
      reason: "Affiliate link in body without disclosure (rule 4).",
      confidence: 0.91,
      decided_at: "2026-04-29T11:50:00Z",
    },
    {
      id: "a3",
      target_type: "submission",
      target_id: "p-pending-2",
      action: "approve",
      reason: "Topic relevance borderline; routed to human queue.",
      confidence: 0.71,
      decided_at: "2026-04-29T14:22:00Z",
    },
    {
      id: "a4",
      target_type: "comment",
      target_id: "c8-2",
      action: "reject",
      reason: "Spam — third near-identical promotional reply this week.",
      confidence: 0.97,
      decided_at: "2026-04-29T07:42:00Z",
      overridden: {
        by: "ada",
        new_action: "approve",
        at: "2026-04-29T08:10:00Z",
        note: "False positive — author is a known contributor; spam classifier mis-fired on URL pattern.",
      },
    },
    {
      id: "a5",
      target_type: "submission",
      target_id: "30",
      action: "approve",
      reason: "Clear practical tip; tagged prompt-caching.",
      confidence: 0.94,
      decided_at: "2026-04-29T10:18:30Z",
    },
  ];
}

