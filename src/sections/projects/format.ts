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
