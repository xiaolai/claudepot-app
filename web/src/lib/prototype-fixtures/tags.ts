/**
 * Tag queries against the prototype fixtures.
 */

import { hotScore, publicVisible, tags } from "./data";
import type { Submission, Tag } from "./types";

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
