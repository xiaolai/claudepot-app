import type { CommentNode, SubmissionType } from "@/lib/prototype-fixtures";

export function ageHours(iso: string): number {
  return (Date.now() - new Date(iso).getTime()) / 3_600_000;
}

export function relativeTime(iso: string): string {
  const hours = Math.max(0, ageHours(iso));
  if (hours < 1) return `${Math.round(hours * 60)}m`;
  if (hours < 24) return `${Math.round(hours)}h`;
  const days = hours / 24;
  if (days < 30) return `${Math.round(days)}d`;
  if (days < 365) return `${Math.round(days / 30)}mo`;
  return `${Math.round(days / 365)}y`;
}

/**
 * Day-only relative time, used where sub-day granularity is noise
 * (e.g., GitHub repo last-update on /projects). Returns "today",
 * "1 day ago", "12 days ago", etc.
 */
export function relativeDays(iso: string): string {
  const days = Math.round(ageHours(iso) / 24);
  if (days <= 0) return "today";
  if (days === 1) return "1 day ago";
  return `${days} days ago`;
}

export function formatDuration(min: number): string {
  if (min < 60) return `${min}m`;
  const h = Math.floor(min / 60);
  const m = min % 60;
  return m === 0 ? `${h}h` : `${h}h ${m}m`;
}

export function totalCommentCount(nodes: CommentNode[]): number {
  return nodes.reduce(
    (acc, n) => acc + 1 + totalCommentCount(n.children),
    0,
  );
}

export const TYPE_LABELS: Record<SubmissionType, string> = {
  news: "news",
  tip: "tip",
  tutorial: "tutorial",
  course: "course",
  article: "article",
  podcast: "podcast",
  interview: "interview",
  tool: "tool",
  discussion: "discussion",
  release: "release",
  paper: "paper",
  workflow: "workflow",
  case_study: "case study",
  prompt_pattern: "prompt pattern",
};
