import { lazy, Suspense } from "react";
import { ErrorBoundary } from "../ErrorBoundary";
import { useOperations } from "../hooks/useOperations";
import { api } from "../api";
import {
  PROJECT_MOVE_PHASES,
  renderProjectMoveResult,
} from "../sections/projects/projectMoveProgress";

const OperationProgressModal = lazy(() =>
  import("../sections/projects/OperationProgressModal").then((m) => ({
    default: m.OperationProgressModal,
  })),
);

/**
 * Shell-level host for the op-progress modal, extracted from
 * AppShell. Reads the active op from the Operations context and
 * mounts the lazy modal when one exists.
 *
 * The modal is wrapped in its own scoped ErrorBoundary (keyed on the
 * op id so a new op resets a prior error state): it's lazy AND fed
 * by backend data, which makes it a realistic crash surface — and
 * without the scoped boundary a render crash here would bubble to
 * main.tsx's full-takeover reload panel and take down the whole
 * shell over one modal.
 */
export function OperationProgressHost(props: {
  /** Called on both complete and error — refresh the pending-journals banner. */
  onTerminal: () => void;
  /** Navigate to Projects → Repair (closes the modal first). */
  onOpenRepair: () => void;
}) {
  const { active: activeOp, close: closeOp } = useOperations();
  if (!activeOp) return null;

  return (
    <ErrorBoundary key={activeOp.opId} label="Operation progress">
      <Suspense fallback={null}>
        <OperationProgressModal
          opId={activeOp.opId}
          title={activeOp.title}
          phases={activeOp.phases ?? PROJECT_MOVE_PHASES}
          fetchStatus={activeOp.fetchStatus ?? api.projectMoveStatus}
          renderResult={activeOp.renderResult ?? renderProjectMoveResult}
          onClose={closeOp}
          onComplete={() => {
            activeOp.onComplete?.();
            props.onTerminal();
          }}
          onError={(detail) => {
            activeOp.onError?.(detail);
            props.onTerminal();
          }}
          onOpenRepair={() => {
            closeOp();
            props.onOpenRepair();
          }}
          onCancel={activeOp.onCancel}
          cancelLabel={activeOp.cancelLabel}
        />
      </Suspense>
    </ErrorBoundary>
  );
}
