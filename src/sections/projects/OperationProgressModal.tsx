import { useCallback, useId, useMemo, useRef, useState, type ReactNode } from "react";
import { useTauriEvent } from "../../hooks/useTauriEvent";
import { Button } from "../../components/primitives/Button";
import { NF } from "../../icons";
import {
  Modal,
  ModalHeader,
  ModalBody,
  ModalFooter,
} from "../../components/primitives/Modal";
import type {
  OperationProgressEvent,
  RunningOpInfo,
} from "../../types";

/**
 * One row in the modal's phase list. `id` is the stable phase id the
 * backend emits (e.g. `"P3"` for project-move, `"S1"` for session-move);
 * `label` is the user-facing string.
 */
export type PhaseSpec = { id: string; label: string };

type PhaseState = "pending" | "running" | "complete" | "error";

/**
 * Subscribes to `op-progress::<opId>` and renders a phase-by-phase
 * progress view. The phase list and the result-rendering policy are
 * passed in by the caller, so the modal is reusable across project
 * move, repair resume/rollback, session move, and any future op that
 * fits the pipe.
 *
 * Closing mid-op only hides the modal — the op keeps running and
 * shows up in the RunningOpStrip.
 */
export function OperationProgressModal({
  opId,
  title,
  phases,
  fetchStatus,
  renderResult,
  onClose,
  onComplete,
  onError,
  onOpenRepair,
  onCancel,
  cancelLabel = "Cancel",
}: {
  opId: string;
  title: string;
  /** Stable list of phase ids + labels expected on this op. */
  phases: PhaseSpec[];
  /** Polled once per terminal event to fetch the structured result.
   *  Project-move passes `api.projectMoveStatus`; session-move passes
   *  `api.sessionMoveStatus`. */
  fetchStatus: (opId: string) => Promise<RunningOpInfo | null>;
  /** Renders the success-state body. Receives the `RunningOpInfo`
   *  returned by `fetchStatus`. Optional — when omitted, only the
   *  "✓ Complete." line shows. */
  renderResult?: (info: RunningOpInfo | null) => ReactNode;
  onClose: () => void;
  /** Fires once when the terminal `op / complete` event lands. */
  onComplete?: () => void;
  /** Fires once on terminal error with the detail string (if any). */
  onError?: (detail: string | null) => void;
  /** Optional: navigate to Repair subview (enables the "Open Repair"
   * button in the error state). If omitted, the button is hidden. */
  onOpenRepair?: (failedJournalId: string | null) => void;
  /** Optional: cancel the in-flight op. When provided, a primary
   *  Cancel button shows in the footer until a terminal event lands.
   *  Implementations are fire-and-forget — the backend's terminal
   *  error event drives the modal back to its terminal state. */
  onCancel?: () => void;
  /** Footer label for the cancel button. Defaults to "Cancel". Login
   *  flows pass "Cancel login" for clarity. */
  cancelLabel?: string;
}) {
  const channel = `op-progress::${opId}`;
  const phaseIds = useMemo(() => phases.map((p) => p.id), [phases]);
  const [phaseStates, setPhaseStates] = useState<Record<string, PhaseState>>(
    () =>
      Object.fromEntries(phaseIds.map((p) => [p, "pending"])) as Record<
        string,
        PhaseState
      >,
  );
  const [sub, setSub] = useState<{
    phase: string;
    done: number;
    total: number;
  } | null>(null);
  const [terminal, setTerminal] = useState<
    | { kind: "complete"; info: RunningOpInfo | null }
    | { kind: "error"; detail: string | null; failedJournalId: string | null }
    | null
  >(null);
  const firedTerminal = useRef(false);
  const headingId = useId();

  // Audit H9: stabilize handler identity so the underlying `listen`
  // subscription attaches once per modal lifetime — preventing the
  // gap-window where a terminal event could be dropped during a
  // resubscribe. All mutable state read in the handler goes through
  // refs.
  const subRef = useRef<{ phase: string; done: number; total: number } | null>(
    null,
  );
  subRef.current = sub;
  const onCompleteRef = useRef(onComplete);
  onCompleteRef.current = onComplete;
  const onErrorRef = useRef(onError);
  onErrorRef.current = onError;
  const fetchStatusRef = useRef(fetchStatus);
  fetchStatusRef.current = fetchStatus;
  const phaseIdSet = useMemo(() => new Set(phaseIds), [phaseIds]);

  const handler = useCallback(
    (event: { payload: OperationProgressEvent }) => {
      const ev = event.payload;
      if (ev.op_id !== opId) return;
      if (ev.phase === "op") {
        if (firedTerminal.current) return;
        firedTerminal.current = true;
        const isComplete = ev.status === "complete";
        fetchStatusRef
          .current(opId)
          .then((info) => {
            if (isComplete) {
              setTerminal({ kind: "complete", info: info ?? null });
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
              setTerminal({ kind: "complete", info: null });
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
      const phase = ev.phase;
      if (!phaseIdSet.has(phase)) return;
      if (typeof ev.done === "number" && typeof ev.total === "number") {
        setSub({ phase, done: ev.done, total: ev.total });
        setPhaseStates((prev) =>
          prev[phase] === "pending" ? { ...prev, [phase]: "running" } : prev,
        );
        return;
      }
      if (ev.status === "complete") {
        setPhaseStates((prev) => ({ ...prev, [phase]: "complete" }));
        if (subRef.current?.phase === phase) setSub(null);
      } else if (ev.status === "error") {
        setPhaseStates((prev) => ({ ...prev, [phase]: "error" }));
      } else {
        setPhaseStates((prev) => ({ ...prev, [phase]: "running" }));
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
          {phases.map((p) => {
            const state = phaseStates[p.id] ?? "pending";
            return (
              <li key={p.id} className={`phase phase-${state}`}>
                <span className="phase-tag" title={`Internal id: ${p.id}`}>
                  {p.label}
                </span>
                <span className="phase-label">{state}</span>
                {sub && sub.phase === p.id && (
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
            {renderResult ? renderResult(terminal.info) : null}
          </div>
        )}
        {terminal?.kind === "error" && (
          isCancelled(terminal.detail) ? (
            <div className="op-terminal info">
              <strong>Cancelled.</strong>
            </div>
          ) : (
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
          )
        )}

      </ModalBody>
      <ModalFooter>
        <Button variant="ghost" onClick={onClose}>
          {terminal ? "Close" : "Run in background"}
        </Button>
        {!terminal && onCancel && (
          <Button
            variant="solid"
            glyph={NF.x}
            onClick={onCancel}
            aria-label={cancelLabel}
          >
            {cancelLabel}
          </Button>
        )}
        {terminal?.kind === "error" &&
          !isCancelled(terminal.detail) &&
          onOpenRepair && (
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

/**
 * The backend signals user-initiated cancellation by way of a terminal
 * error whose detail contains "cancel" (e.g. `claude auth login was
 * cancelled by the user`, `move was cancelled`). The modal renders that
 * as a calmer "Cancelled." state instead of a red "Error." — the user
 * just clicked Cancel, they don't need to be alarmed.
 */
function isCancelled(detail: string | null | undefined): boolean {
  return typeof detail === "string" && /cancel/i.test(detail);
}
