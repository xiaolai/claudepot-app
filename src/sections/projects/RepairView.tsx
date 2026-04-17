import { useCallback, useEffect, useState } from "react";
import { ArrowLeft, Wrench } from "lucide-react";
import { api } from "../../api";
import { useOperations } from "../../hooks/useOperations";
import type { JournalEntry } from "../../types";
import { RepairEntry } from "./RepairEntry";
import { RepairConfirmDialogs, type PendingAction } from "./RepairConfirmDialogs";
import { ConfirmDangerousAction } from "../../components/ConfirmDangerousAction";

export function RepairView({
  onBack,
  onOpTerminated,
  embedded,
}: {
  onBack: () => void;
  onOpTerminated?: () => void;
  embedded?: boolean;
}) {
  const [entries, setEntries] = useState<JournalEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState<PendingAction | null>(null);
  const [breakLockTarget, setBreakLockTarget] = useState<JournalEntry | null>(
    null,
  );
  const [toast, setToast] = useState<string | null>(null);
  const { open: openOpModal } = useOperations();

  const refresh = useCallback(() => {
    setLoading(true);
    api.repairList()
      .then((es) => {
        setEntries([...es].sort((a, b) => b.started_unix_secs - a.started_unix_secs));
        setLoading(false);
        setError(null);
      })
      .catch((e) => { setError(String(e)); setLoading(false); });
  }, []);

  useEffect(() => { refresh(); }, [refresh]);

  const afterTerminal = () => { setToast("Done."); refresh(); onOpTerminated?.(); };

  const runResume = async (entry: JournalEntry) => {
    setPending(null);
    try {
      const opId = await api.repairResumeStart(entry.id);
      openOpModal({ opId, title: `Resuming ${entry.id}`, onComplete: afterTerminal, onError: afterTerminal });
    } catch (e) { setToast(`Resume failed: ${e}`); }
  };

  const runRollback = async (entry: JournalEntry) => {
    setPending(null);
    try {
      const opId = await api.repairRollbackStart(entry.id);
      openOpModal({ opId, title: `Rolling back ${entry.id}`, onComplete: afterTerminal, onError: afterTerminal });
    } catch (e) { setToast(`Rollback failed: ${e}`); }
  };

  const runAbandon = async (entry: JournalEntry) => {
    setPending(null);
    try {
      await api.repairAbandon(entry.id);
      setToast(`Abandoned ${entry.id}.`);
      refresh();
      onOpTerminated?.();
    } catch (e) { setToast(`Abandon failed: ${e}`); }
  };

  const runBreakLock = async (entry: JournalEntry) => {
    setBreakLockTarget(null);
    try {
      const outcome = await api.repairBreakLock(entry.old_path);
      setToast(
        `Lock broken — prior owner PID ${outcome.prior_pid} on ${outcome.prior_hostname}. Audit saved.`,
      );
      refresh();
      onOpTerminated?.();
    } catch (e) {
      setToast(`Break lock failed: ${e}`);
    }
  };

  const Wrapper = embedded ? "div" : "main";

  return (
    <Wrapper className={embedded ? "repair-view-embedded" : "content repair-view"}>
      {!embedded && (
        <header className="repair-header">
          <button type="button" className="icon-btn" onClick={onBack}
            aria-label="Back to Projects" title="Back to Projects">
            <ArrowLeft />
          </button>
          <h2><Wrench /> Repair</h2>
        </header>
      )}

      {loading && entries.length === 0 && (
        <div className="skeleton-container"><div className="skeleton skeleton-card" /></div>
      )}
      {error && (
        <div className="banner warn" role="alert">
          <div><strong>Couldn't load repair queue.</strong> <span className="mono">{error}</span></div>
        </div>
      )}
      {!loading && !error && entries.length === 0 && (
        <div className="empty">
          <Wrench size={32} strokeWidth={1} />
          <h2>All clear</h2>
          <p className="muted">No pending rename journals.</p>
        </div>
      )}

      {entries.length > 0 && (
        <ul className="repair-list">
          {entries.map((e) => (
            <RepairEntry key={e.id} entry={e}
              onResume={() => setPending({ kind: "resume", entry: e })}
              onRollback={() => setPending({ kind: "rollback", entry: e })}
              onAbandon={() => setPending({ kind: "abandon", entry: e })}
              onBreakLock={() => setBreakLockTarget(e)} />
          ))}
        </ul>
      )}

      <RepairConfirmDialogs pending={pending} onCancel={() => setPending(null)}
        onResume={runResume} onRollback={runRollback} onAbandon={runAbandon} />

      {breakLockTarget && (
        <ConfirmDangerousAction
          title="Break lock?"
          confirmLabel="Break lock"
          consequences={
            <>
              <p>
                Force-breaks the lock file for this journal and writes an
                audit record. The prior owner (if still alive) will fail
                on its next write.
              </p>
              <p className="mono small selectable">
                {breakLockTarget.old_path}
              </p>
              <p className="muted small">
                Safe when the journal is stale (≥24 h). Don't break a
                running lock — that can corrupt in-flight state.
              </p>
            </>
          }
          onCancel={() => setBreakLockTarget(null)}
          onConfirm={() => runBreakLock(breakLockTarget)}
        />
      )}

      {toast && (
        <div className="inline-toast" role="status" onClick={() => setToast(null)}>{toast}</div>
      )}
    </Wrapper>
  );
}
