import {
  createContext,
  useCallback,
  useContext,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import type { PhaseSpec } from "../sections/projects/OperationProgressModal";
import type { RunningOpInfo } from "../types";

/**
 * Handle to a currently-visible operation-progress modal. The shell
 * renders at most one such modal; starting a new op replaces any
 * prior handle (which only hides that modal — the prior op keeps
 * running in the background strip).
 */
export interface OpHandle {
  opId: string;
  title: string;
  /** Phase ids + labels expected for this op. Defaults to the
   *  project-move phase set when omitted, for backward compat with
   *  call sites that haven't migrated yet. */
  phases?: PhaseSpec[];
  /** How to fetch the current `RunningOpInfo` snapshot when the
   *  terminal event lands. Defaults to `api.projectMoveStatus`. */
  fetchStatus?: (opId: string) => Promise<RunningOpInfo | null>;
  /** Renders the success-state body. Defaults to
   *  `renderProjectMoveResult`. */
  renderResult?: (info: RunningOpInfo | null) => ReactNode;
  /** Fired on terminal complete event. */
  onComplete?: () => void;
  /** Fired on terminal error event, with detail. */
  onError?: (detail: string | null) => void;
}

interface OperationsContextValue {
  active: OpHandle | null;
  /** Start showing a progress modal for this op. */
  open: (handle: OpHandle) => void;
  /** Hide the modal (op keeps running if not terminal). */
  close: () => void;
}

const OperationsContext = createContext<OperationsContextValue | null>(null);

/**
 * Shell-level provider for the active op-progress modal. Any child
 * can call `open({opId, title})` to surface the modal; closing
 * backgrounds the op (does not cancel — plan §2.5).
 */
export function OperationsProvider({ children }: { children: ReactNode }) {
  const [active, setActive] = useState<OpHandle | null>(null);

  const open = useCallback((handle: OpHandle) => setActive(handle), []);
  const close = useCallback(() => setActive(null), []);

  const value = useMemo(
    () => ({ active, open, close }),
    [active, open, close],
  );

  return (
    <OperationsContext.Provider value={value}>
      {children}
    </OperationsContext.Provider>
  );
}

export function useOperations(): OperationsContextValue {
  const ctx = useContext(OperationsContext);
  if (!ctx) {
    throw new Error("useOperations must be used inside OperationsProvider");
  }
  return ctx;
}
