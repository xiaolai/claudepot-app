import { useCallback, useEffect, useRef, useState } from "react";
import { useTauriEvent } from "../../hooks/useTauriEvent";
import type { OperationProgressEvent } from "../../types";

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
}: {
  opId: string;
  title: string;
  onClose: () => void;
  /** Fires once when the terminal `op / complete` event lands. */
  onComplete?: () => void;
  /** Fires once on terminal error with the detail string (if any). */
  onError?: (detail: string | null) => void;
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
    { kind: "complete" } | { kind: "error"; detail: string | null } | null
  >(null);
  const firedTerminal = useRef(false);
  const headingId = useRef(
    `op-progress-heading-${Math.random().toString(36).slice(2, 9)}`,
  );

  const handler = useCallback(
    (event: { payload: OperationProgressEvent }) => {
      const ev = event.payload;
      if (ev.op_id !== opId) return;
      if (ev.phase === "op") {
        // Terminal event.
        if (firedTerminal.current) return;
        firedTerminal.current = true;
        if (ev.status === "complete") {
          setTerminal({ kind: "complete" });
          onComplete?.();
        } else {
          setTerminal({ kind: "error", detail: ev.detail ?? null });
          onError?.(ev.detail ?? null);
        }
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
        if (sub?.phase === phase) setSub(null);
      } else if (ev.status === "error") {
        setPhases((prev) => ({ ...prev, [phase]: "error" }));
      } else {
        setPhases((prev) => ({ ...prev, [phase]: "running" }));
      }
    },
    [opId, onComplete, onError, sub?.phase],
  );

  useTauriEvent<OperationProgressEvent>(channel, handler);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  return (
    <div className="modal-backdrop" role="presentation">
      <div
        className="modal op-progress-modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby={headingId.current}
      >
        <h2 id={headingId.current}>{title}</h2>

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
          </div>
        )}
        {terminal?.kind === "error" && (
          <div className="op-terminal bad">
            <strong>Error.</strong>{" "}
            <span className="mono small">{terminal.detail ?? "unknown"}</span>
          </div>
        )}

        <div className="modal-actions">
          <button type="button" onClick={onClose}>
            {terminal ? "Close" : "Run in background"}
          </button>
        </div>
      </div>
    </div>
  );
}
