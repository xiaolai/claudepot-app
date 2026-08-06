import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { Icon } from "../../components/Icon";
import type { JournalEntry, JournalStatus } from "../../types";

/** Status → user-facing copy, resolved at render time. */
function statusCopy(t: TFunction<"projects">, s: JournalStatus): string {
  switch (s) {
    case "running": return t("repair.statusRunning");
    case "pending": return t("repair.statusPending");
    case "stale": return t("repair.statusStale");
    case "abandoned": return t("repair.statusAbandoned");
  }
}

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
  const { t } = useTranslation("projects");
  return (
    <li
      className={`repair-entry status-${e.status}`}
      aria-label={t("repair.journalAria", {
        id: e.id,
        status: statusCopy(t, e.status),
      })}
    >
      <div className="repair-entry-head">
        <span className={`tag ${statusClass(e.status)}`}>
          {statusCopy(t, e.status)}
        </span>
        <span className="mono small muted">{e.id}</span>
      </div>
      <div className="repair-entry-paths">
        <span className="mono small selectable">{e.old_path}</span>
        <span className="muted"> → </span>
        <span className="mono small selectable">{e.new_path}</span>
      </div>
      <div className="repair-entry-meta muted small">
        {t("repair.startedMeta", {
          date: e.started_at,
          phases: e.phases_completed.join(", ") || t("repair.phasesNone"),
        })}
      </div>
      {e.last_error && (
        <div className="repair-entry-error bad small">
          {t("repair.lastError", { error: e.last_error })}
        </div>
      )}
      {e.status !== "abandoned" && e.status !== "running" && (
        // A `running` journal is owned by an in-flight rename; offering
        // Resume/Rollback/Abandon here lets the user race that operation
        // and corrupt the in-memory state machine. Only `pending` and
        // `stale` (lock present but no live owner) entries get the
        // mutating actions; `running` is render-only.
        <div className="repair-entry-actions">
          <button type="button" className="btn" title={t("repair.resumeTitle")} onClick={onResume}>
            <Icon name="rotate-ccw" />{t("repair.resume")}
          </button>
          <button type="button" className="btn" title={t("repair.rollbackTitle")} onClick={onRollback}>
            <Icon name="undo" />{t("repair.rollback")}
          </button>
          {e.status === "stale" && onBreakLock && (
            <button
              type="button"
              className="btn warn"
              title={t("repair.breakLockBtnTitle")}
              onClick={onBreakLock}
            >
              <Icon name="unlock" />{t("repair.breakLock")}
            </button>
          )}
          <button type="button" className="btn danger" title={t("repair.abandonTitle")} onClick={onAbandon}>
            <Icon name="ban" />{t("repair.abandon")}
          </button>
        </div>
      )}
    </li>
  );
}
