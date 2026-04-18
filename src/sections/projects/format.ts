/**
 * Shared formatting helpers for the projects subtree.
 *
 * Audit Low: formatSize was duplicated across ProjectsList, ProjectDetail,
 * and CleanOrphansModal. Moved here so formatting changes (precision,
 * locale, unit cap) happen in exactly one place.
 */

export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024)
    return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  return `${(bytes / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;
const WEEK = 7 * DAY;

/**
 * Compact "N unit ago" for a past instant in ms. Coarsest unit wins;
 * always rounded down so "5 minutes and 50 seconds ago" → "5m ago".
 * For instants in the future (clock skew) returns "just now" rather
 * than confusing the user with a negative magnitude.
 */
export function formatRelativeTime(ms: number): string {
  const diff = Date.now() - ms;
  if (diff < MINUTE) return "just now";
  if (diff < HOUR) return `${Math.floor(diff / MINUTE)}m ago`;
  if (diff < DAY) return `${Math.floor(diff / HOUR)}h ago`;
  if (diff < WEEK) return `${Math.floor(diff / DAY)}d ago`;
  return `${Math.floor(diff / WEEK)}w ago`;
}
