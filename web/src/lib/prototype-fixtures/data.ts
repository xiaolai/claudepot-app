/**
 * Fixture data load + visibility helpers shared across the per-domain
 * query modules. Kept private to the directory — exported only to
 * sibling files via relative imports.
 *
 * `ageHours` is computed relative to real-time `Date.now()`. The
 * prototype previously hardcoded NOW = "2026-04-29T15:00:00Z" so
 * fixture timestamps read as "1h ago"; now that the app reads from
 * the live DB (real timestamps from gh CLI, real user submissions),
 * Date.now() is correct. Fixtures rendered "fresh" during prototype
 * iteration are now correctly older. The helper itself lives in
 * @/lib/format.
 */

import submissionsRaw from "../../../design/fixtures/submissions.json";
import commentsRaw from "../../../design/fixtures/comments.json";
import usersRaw from "../../../design/fixtures/users.json";
import projectsRaw from "../../../design/fixtures/projects.json";
import tagsRaw from "../../../design/fixtures/tags.json";
import { ageHours } from "@/lib/format";
import { effectiveState } from "@/lib/moderation-fixtures";

import type { CommentNode, Project, Submission, Tag, User } from "./types";

export const submissions = submissionsRaw as Submission[];
export const commentsByPost = commentsRaw as Record<string, CommentNode[]>;
export const users = usersRaw as User[];
export const projects = projectsRaw as Project[];
export const tags = tagsRaw as Tag[];

/** Public-visible submissions: approved only. */
export function publicVisible(): Submission[] {
  return submissions.filter((s) => effectiveState(s) === "approved");
}

export function hotScore(s: Submission): number {
  const net = s.upvotes - s.downvotes;
  return Math.max(net - 1, 0) / Math.pow(ageHours(s.submitted_at) + 2, 1.8);
}
