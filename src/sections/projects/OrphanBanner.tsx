import { Icon } from "../../components/Icon";
import type { OrphanedProject } from "../../types";
import { formatSize } from "./format";

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
  const sizeLabel = formatSize(byteTotal);
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

