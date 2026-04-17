import { RotateCcw, Undo2, Ban, Unlock } from "lucide-react";
import type { JournalEntry, JournalStatus } from "../../types";

const STATUS_COPY: Record<JournalStatus, string> = {
  running: "running",
  pending: "pending",
  stale: "stale ≥24h",
  abandoned: "abandoned",
};

function statusClass(s: JournalStatus): string {
  switch (s) {
    case "running": return "ok";
    case "pending": return "";
    case "stale": return "warn";
    case "abandoned": return "muted";
  }
}

export function RepairEntry({
  entry: e,
  onResume,
  onRollback,
  onAbandon,
  onBreakLock,
}: {
  entry: JournalEntry;
  onResume: () => void;
  onRollback: () => void;
  onAbandon: () => void;
  /**
   * Force-break the lock file owned by this journal's old_path.
   * Surfaced only for stale entries, since a running entry's lock is
   * legitimate and breaking it mid-run corrupts state. Rendered as a
   * destructive action with a confirm dialog in the caller.
   */
  onBreakLock?: () => void;
}) {
  return (
    <li
      className={`repair-entry status-${e.status}`}
      aria-label={`Journal ${e.id} — ${STATUS_COPY[e.status]}`}
    >
      <div className="repair-entry-head">
        <span className={`tag ${statusClass(e.status)}`}>
          {STATUS_COPY[e.status]}
        </span>
        <span className="mono small muted">{e.id}</span>
      </div>
      <div className="repair-entry-paths">
        <span className="mono small selectable">{e.old_path}</span>
        <span className="muted"> → </span>
        <span className="mono small selectable">{e.new_path}</span>
      </div>
      <div className="repair-entry-meta muted small">
        started {e.started_at} · phases [
        {e.phases_completed.join(", ") || "none"}]
      </div>
      {e.last_error && (
        <div className="repair-entry-error bad small">
          last error: {e.last_error}
        </div>
      )}
      {e.status !== "abandoned" && (
        <div className="repair-entry-actions">
          <button type="button" title="Re-run the original rename" onClick={onResume}>
            <RotateCcw /> Resume
          </button>
          <button type="button" title="Reverse the rename (runs new → old)" onClick={onRollback}>
            <Undo2 /> Rollback
          </button>
          {e.status === "stale" && onBreakLock && (
            <button
              type="button"
              className="warn"
              title="Force-break the stale lock file so resume can proceed"
              onClick={onBreakLock}
            >
              <Unlock /> Break lock
            </button>
          )}
          <button type="button" className="danger" title="Stop nagging about this journal" onClick={onAbandon}>
            <Ban /> Abandon
          </button>
        </div>
      )}
    </li>
  );
}
