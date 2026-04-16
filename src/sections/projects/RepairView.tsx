import { useCallback, useEffect, useState } from "react";
import {
  ArrowCounterClockwise,
  ArrowLeft,
  ArrowUUpLeft,
  Prohibit,
  Wrench,
} from "@phosphor-icons/react";
import { api } from "../../api";
import { ConfirmDangerousAction } from "../../components/ConfirmDangerousAction";
import { OperationProgressModal } from "./OperationProgressModal";
import type { JournalEntry, JournalStatus } from "../../types";

const STATUS_COPY: Record<JournalStatus, string> = {
  running: "running",
  pending: "pending",
  stale: "stale ≥24h",
  abandoned: "abandoned",
};

type PendingAction =
  | { kind: "resume"; entry: JournalEntry }
  | { kind: "rollback"; entry: JournalEntry }
  | { kind: "abandon"; entry: JournalEntry };

/**
 * Pending-journal list with actions wired to the mutating Tauri
 * commands. Each destructive action goes through ConfirmDangerousAction;
 * Abandon uses type-to-confirm because it removes the entry from the
 * nag queue permanently.
 *
 * Resume / Rollback fire `*_start` commands that return an op_id, then
 * OperationProgressModal subscribes to `op-progress::<op_id>`.
 */
export function RepairView({
  onBack,
  onOpTerminated,
}: {
  onBack: () => void;
  /** Fired when an op completes/errors so the parent can refresh banners. */
  onOpTerminated?: () => void;
}) {
  const [entries, setEntries] = useState<JournalEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState<PendingAction | null>(null);
  const [activeOp, setActiveOp] = useState<
    { opId: string; title: string } | null
  >(null);
  const [toast, setToast] = useState<string | null>(null);

  const refresh = useCallback(() => {
    setLoading(true);
    api
      .repairList()
      .then((es) => {
        setEntries(es);
        setLoading(false);
        setError(null);
      })
      .catch((e) => {
        setError(String(e));
        setLoading(false);
      });
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const runResume = async (entry: JournalEntry) => {
    setPending(null);
    try {
      const opId = await api.repairResumeStart(entry.id);
      setActiveOp({ opId, title: `Resuming ${entry.id}` });
    } catch (e) {
      setToast(`Resume failed: ${e}`);
    }
  };
  const runRollback = async (entry: JournalEntry) => {
    setPending(null);
    try {
      const opId = await api.repairRollbackStart(entry.id);
      setActiveOp({ opId, title: `Rolling back ${entry.id}` });
    } catch (e) {
      setToast(`Rollback failed: ${e}`);
    }
  };
  const runAbandon = async (entry: JournalEntry) => {
    setPending(null);
    try {
      await api.repairAbandon(entry.id);
      setToast(`Abandoned ${entry.id}.`);
      refresh();
      onOpTerminated?.();
    } catch (e) {
      setToast(`Abandon failed: ${e}`);
    }
  };

  return (
    <main className="content repair-view">
      <header className="repair-header">
        <button
          type="button"
          className="icon-btn"
          onClick={onBack}
          aria-label="Back to Projects"
          title="Back to Projects"
        >
          <ArrowLeft />
        </button>
        <h2>
          <Wrench /> Repair
        </h2>
      </header>

      {loading && entries.length === 0 && (
        <div className="skeleton-container">
          <div className="skeleton skeleton-card" />
        </div>
      )}

      {error && (
        <div className="banner warn" role="alert">
          <div>
            <strong>Couldn't load repair queue.</strong>{" "}
            <span className="mono">{error}</span>
          </div>
        </div>
      )}

      {!loading && !error && entries.length === 0 && (
        <div className="empty">
          <Wrench size={32} weight="thin" />
          <h2>All clear</h2>
          <p className="muted">No pending rename journals.</p>
        </div>
      )}

      {entries.length > 0 && (
        <ul className="repair-list">
          {entries.map((e) => (
            <li
              key={e.id}
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
                  <button
                    type="button"
                    title="Re-run the original rename"
                    onClick={() => setPending({ kind: "resume", entry: e })}
                  >
                    <ArrowCounterClockwise /> Resume
                  </button>
                  <button
                    type="button"
                    title="Reverse the rename (runs new → old)"
                    onClick={() => setPending({ kind: "rollback", entry: e })}
                  >
                    <ArrowUUpLeft /> Rollback
                  </button>
                  <button
                    type="button"
                    className="danger"
                    title="Stop nagging about this journal"
                    onClick={() => setPending({ kind: "abandon", entry: e })}
                  >
                    <Prohibit /> Abandon
                  </button>
                </div>
              )}
            </li>
          ))}
        </ul>
      )}

      {pending?.kind === "resume" && (
        <ConfirmDangerousAction
          title="Resume rename?"
          confirmLabel="Resume"
          danger={false}
          consequences={
            <>
              <p>Re-runs the move pipeline. Phases are idempotent.</p>
              <p className="mono small muted">
                {pending.entry.old_path} → {pending.entry.new_path}
              </p>
            </>
          }
          onCancel={() => setPending(null)}
          onConfirm={() => runResume(pending.entry)}
        />
      )}

      {pending?.kind === "rollback" && (
        <ConfirmDangerousAction
          title="Rollback rename?"
          confirmLabel="Rollback"
          consequences={
            <>
              <p>Runs the reverse move (new → old).</p>
              {pending.entry.snapshot_paths.length > 0 && (
                <div className="muted small">
                  <strong>Snapshots of destructive-phase targets:</strong>
                  <ul>
                    {pending.entry.snapshot_paths.map((s) => (
                      <li key={s} className="mono">
                        {s}
                      </li>
                    ))}
                  </ul>
                  Snapshots are NOT auto-restored.
                </div>
              )}
            </>
          }
          onCancel={() => setPending(null)}
          onConfirm={() => runRollback(pending.entry)}
        />
      )}

      {pending?.kind === "abandon" && (
        <ConfirmDangerousAction
          title="Abandon journal?"
          confirmLabel="Abandon"
          typeToConfirm="ABANDON"
          consequences={
            <>
              <p>
                Writes a <code className="mono">.abandoned.json</code> sidecar.
                Future runs will no longer nag about this journal.
              </p>
              <p className="muted small">
                Audit trail is preserved; the journal itself is kept on disk
                for inspection.
              </p>
            </>
          }
          onCancel={() => setPending(null)}
          onConfirm={() => runAbandon(pending.entry)}
        />
      )}

      {activeOp && (
        <OperationProgressModal
          opId={activeOp.opId}
          title={activeOp.title}
          onClose={() => setActiveOp(null)}
          onComplete={() => {
            setToast("Complete.");
            refresh();
            onOpTerminated?.();
          }}
          onError={(detail) => {
            setToast(`Failed: ${detail ?? "unknown"}`);
            refresh();
            onOpTerminated?.();
          }}
        />
      )}

      {toast && (
        <div className="inline-toast" role="status" onClick={() => setToast(null)}>
          {toast}
        </div>
      )}
    </main>
  );
}

function statusClass(s: JournalStatus): string {
  switch (s) {
    case "running":
      return "ok";
    case "pending":
      return "";
    case "stale":
      return "warn";
    case "abandoned":
      return "muted";
  }
}
