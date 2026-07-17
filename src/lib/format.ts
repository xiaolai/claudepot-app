// Shared numeric formatting helpers.
//
// `formatSize` is the canonical byte formatter (B / KB / MB / GB
// ladder, one decimal below GB, two at GB). It was consolidated here
// from `sections/projects/format.ts` after byte-identical copies
// (capped at MB) accumulated in GcCard and useSettingsActions
// (audit 2026-07 F10). `sections/projects/format.ts` re-exports it
// so the projects subtree keeps its import path.

export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024)
    return `${(bytes / 1024 / 1024).toFixed(1)} MB`;
  return `${(bytes / 1024 / 1024 / 1024).toFixed(2)} GB`;
}
