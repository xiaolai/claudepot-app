import { useCallback, useId, useRef, useState } from "react";
import { useTauriEvent } from "../../hooks/useTauriEvent";
import { api } from "../../api";
import { Button } from "../../components/primitives/Button";
import {
  Modal,
  ModalHeader,
  ModalBody,
  ModalFooter,
} from "../../components/primitives/Modal";
import type {
  MoveResultSummary,
  OperationProgressEvent,
} from "../../types";

const PHASES = ["P3", "P4", "P5", "P6", "P7", "P8", "P9"] as const;
type Phase = (typeof PHASES)[number];

type PhaseState = "pending" | "running" | "complete" | "error";

/**
 * Subscribes to `op-progress::<opId>` and renders a phase-by-phase
 * progress view. Serves resume, rollback, and (in Step 6) fresh
 * rename. Closing mid-op only hides the modal — the op keeps running
 * and shows up in the RunningOpStrip.
 */
export function OperationProgressModal({
  opId,
  title,
  onClose,
  onComplete,
  onError,
  onOpenRepair,
}: {
  opId: string;
  title: string;
  onClose: () => void;
  /** Fires once when the terminal `op / complete` event lands. */
  onComplete?: () => void;
  /** Fires once on terminal error with the detail string (if any). */
  onError?: (detail: string | null) => void;
  /** Optional: navigate to Repair subview (enables the "Open Repair"
   * button in the error state). If omitted, the button is hidden. */
  onOpenRepair?: (failedJournalId: string | null) => void;
}) {
  const channel = `op-progress::${opId}`;
  const [phases, setPhases] = useState<Record<Phase, PhaseState>>(
    () =>
      Object.fromEntries(PHASES.map((p) => [p, "pending"])) as Record<
        Phase,
        PhaseState
      >,
  );
  const [sub, setSub] = useState<{
    phase: Phase;
    done: number;
    total: number;
  } | null>(null);
  const [terminal, setTerminal] = useState<
    | { kind: "complete"; result: MoveResultSummary | null }
    | { kind: "error"; detail: string | null; failedJournalId: string | null }
    | null
  >(null);
  const firedTerminal = useRef(false);
  const headingId = useId();

  // Audit H9: the handler used to close over `sub?.phase`, making its
  // identity change every time a sub-progress update arrived. The
  // inner `useTauriEvent(channel, handler)` subscribed by
  // [channel, handler], so the subscription torn down and re-attached
  // on every phase transition. Because `listen()` is async, that
  // created a gap in which a terminal `op` event could land and be
  // missed, leaving the modal stuck in a non-terminal state.
  //
  // Fix: stabilize handler identity. All mutable state accessed by
  // the handler goes through refs, so the deps array shrinks to [].
  // Callback identity is constant for the modal's lifetime; the
  // subscription attaches once on mount and detaches once on unmount.
  const subRef = useRef<{ phase: Phase; done: number; total: number } | null>(null);
  subRef.current = sub;
  const onCompleteRef = useRef(onComplete);
  onCompleteRef.current = onComplete;
  const onErrorRef = useRef(onError);
  onErrorRef.current = onError;

  const handler = useCallback(
    (event: { payload: OperationProgressEvent }) => {
      const ev = event.payload;
      if (ev.op_id !== opId) return;
      if (ev.phase === "op") {
        if (firedTerminal.current) return;
        firedTerminal.current = true;
        const isComplete = ev.status === "complete";
        api
          .projectMoveStatus(opId)
          .then((info) => {
            if (isComplete) {
              setTerminal({
                kind: "complete",
                result: info?.move_result ?? null,
              });
              onCompleteRef.current?.();
            } else {
              setTerminal({
                kind: "error",
                detail: ev.detail ?? info?.last_error ?? null,
                failedJournalId: info?.failed_journal_id ?? null,
              });
              onErrorRef.current?.(ev.detail ?? null);
            }
          })
          .catch(() => {
            if (isComplete) {
              setTerminal({ kind: "complete", result: null });
              onCompleteRef.current?.();
            } else {
              setTerminal({
                kind: "error",
                detail: ev.detail ?? null,
                failedJournalId: null,
              });
              onErrorRef.current?.(ev.detail ?? null);
            }
          });
        return;
      }
      const phase = ev.phase as Phase;
      if (!PHASES.includes(phase)) return;
      if (typeof ev.done === "number" && typeof ev.total === "number") {
        setSub({ phase, done: ev.done, total: ev.total });
        setPhases((prev) =>
          prev[phase] === "pending" ? { ...prev, [phase]: "running" } : prev,
        );
        return;
      }
      if (ev.status === "complete") {
        setPhases((prev) => ({ ...prev, [phase]: "complete" }));
        if (subRef.current?.phase === phase) setSub(null);
      } else if (ev.status === "error") {
        setPhases((prev) => ({ ...prev, [phase]: "error" }));
      } else {
        setPhases((prev) => ({ ...prev, [phase]: "running" }));
      }
    },
    // Intentionally empty — handler reads mutable state via refs so
    // its identity stays stable for the modal's lifetime.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [opId],
  );

  useTauriEvent<OperationProgressEvent>(channel, handler);

  return (
    <Modal open onClose={onClose} aria-labelledby={headingId}>
      <ModalHeader title={title} id={headingId} onClose={onClose} />
      <ModalBody>
        <ul className="phase-list">
          {PHASES.map((p) => {
            const state = phases[p];
            return (
              <li key={p} className={`phase phase-${state}`}>
                <span className="phase-tag">{p}</span>
                <span className="phase-label">{state}</span>
                {sub && sub.phase === p && (
                  <span className="phase-progress mono small muted">
                    {" "}({sub.done}/{sub.total})
                  </span>
                )}
              </li>
            );
          })}
        </ul>

        {terminal?.kind === "complete" && (
          <div className="op-terminal ok">
            <strong>✓ Complete.</strong>
            {terminal.result && (
              <ul className="op-terminal-detail">
                {terminal.result.actual_dir_moved && (
                  <li>Source directory moved.</li>
                )}
                {terminal.result.cc_dir_renamed && (
                  <li>
                    CC project dir renamed;{" "}
                    {terminal.result.jsonl_files_modified} of{" "}
                    {terminal.result.jsonl_files_scanned} jsonl file
                    {terminal.result.jsonl_files_scanned === 1 ? "" : "s"}{" "}
                    rewritten.
                  </li>
                )}
                {terminal.result.memory_dir_moved && (
                  <li>Auto-memory directory moved.</li>
                )}
                {terminal.result.config_had_collision &&
                  terminal.result.config_snapshot_path && (
                    <li>
                      Pre-existing data preserved at{" "}
                      <code className="mono small">
                        {terminal.result.config_snapshot_path}
                      </code>
                      . Retained 30 days.
                    </li>
                  )}
                {terminal.result.warnings.length > 0 && (
                  <li className="muted small">
                    Warnings:
                    <ul>
                      {terminal.result.warnings.map((w, i) => (
                        <li key={i}>{w}</li>
                      ))}
                    </ul>
                  </li>
                )}
              </ul>
            )}
          </div>
        )}
        {terminal?.kind === "error" && (
          <div className="op-terminal bad">
            <strong>Error.</strong>{" "}
            <span className="mono small">{terminal.detail ?? "unknown"}</span>
            {terminal.failedJournalId && (
              <p className="small muted">
                Journal id:{" "}
                <code className="mono">{terminal.failedJournalId}</code>
              </p>
            )}
          </div>
        )}

      </ModalBody>
      <ModalFooter>
        <Button variant="ghost" onClick={onClose}>
          {terminal ? "Close" : "Run in background"}
        </Button>
        {terminal?.kind === "error" && onOpenRepair && (
          <Button
            variant="solid"
            onClick={() => onOpenRepair(terminal.failedJournalId)}
          >
            Open Repair
          </Button>
        )}
      </ModalFooter>
    </Modal>
  );
}
