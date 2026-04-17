import { ConfirmDangerousAction } from "../../components/ConfirmDangerousAction";
import type { JournalEntry } from "../../types";

type PendingAction =
  | { kind: "resume"; entry: JournalEntry }
  | { kind: "rollback"; entry: JournalEntry }
  | { kind: "abandon"; entry: JournalEntry };

export type { PendingAction };

export function RepairConfirmDialogs({
  pending,
  onCancel,
  onResume,
  onRollback,
  onAbandon,
}: {
  pending: PendingAction | null;
  onCancel: () => void;
  onResume: (entry: JournalEntry) => void;
  onRollback: (entry: JournalEntry) => void;
  onAbandon: (entry: JournalEntry) => void;
}) {
  if (!pending) return null;

  if (pending.kind === "resume") {
    return (
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
        onCancel={onCancel}
        onConfirm={() => onResume(pending.entry)}
      />
    );
  }

  if (pending.kind === "rollback") {
    return (
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
                    <li key={s} className="mono">{s}</li>
                  ))}
                </ul>
                Snapshots are NOT auto-restored.
              </div>
            )}
          </>
        }
        onCancel={onCancel}
        onConfirm={() => onRollback(pending.entry)}
      />
    );
  }

  return (
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
            Audit trail is preserved; the journal itself is kept on disk.
          </p>
        </>
      }
      onCancel={onCancel}
      onConfirm={() => onAbandon(pending.entry)}
    />
  );
}
