import { Icon } from "../../components/Icon";
import type { OrphanedProject } from "../../types";

/**
 * Persistent banner surfaced whenever `~/.claude/projects/` contains
 * slugs whose original cwd no longer exists on disk. The primary
 * trigger: a git worktree was removed while it had active CC sessions.
 *
 * Feedback-ladder role: "Persistent state requiring attention (warn)".
 * Banner, not toast — the state persists until the user acts on it.
 */
export function OrphanBanner({
  orphans,
  onAdopt,
}: {
  orphans: OrphanedProject[];
  onAdopt: () => void;
}) {
  if (orphans.length === 0) return null;

  const sessionTotal = orphans.reduce((n, o) => n + o.sessionCount, 0);
  const byteTotal = orphans.reduce((n, o) => n + o.totalSizeBytes, 0);
  const sizeLabel = formatBytes(byteTotal);
  const label =
    orphans.length === 1
      ? `1 orphaned project (${sessionTotal} session${sessionTotal === 1 ? "" : "s"}, ${sizeLabel})`
      : `${orphans.length} orphaned projects (${sessionTotal} sessions, ${sizeLabel})`;

  return (
    <div className="banner banner-warn" role="alert">
      <Icon name="alert-triangle" size={14} />
      <div className="banner-body">
        <strong>{label}</strong>
        <span className="banner-hint">
          Their original cwd no longer exists. Adopt them into a live
          project to keep them resumable.
        </span>
      </div>
      <button className="btn" onClick={onAdopt}>
        Review &amp; adopt…
      </button>
    </div>
  );
}

function formatBytes(n: number): string {
  const KB = 1024, MB = KB * 1024, GB = MB * 1024;
  if (n >= GB) return `${(n / GB).toFixed(1)} GB`;
  if (n >= MB) return `${(n / MB).toFixed(1)} MB`;
  if (n >= KB) return `${(n / KB).toFixed(1)} KB`;
  return `${n} B`;
}
