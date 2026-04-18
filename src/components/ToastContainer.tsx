import { Icon } from "./Icon";
import type { Toast } from "../hooks/useToasts";

export function ToastContainer({
  toasts,
  onDismiss,
}: {
  toasts: Toast[];
  onDismiss: (id: number) => void;
}) {
  return (
    <div className="toasts" aria-live="polite">
      {toasts.map((t) => (
        <div key={t.id} className={`toast ${t.kind} ${t.exiting ? "exiting" : ""}`}>
          <span className="toast-text">{t.text}</span>
          {t.onUndo && (
            <button
              className="toast-undo"
              onClick={() => {
                t.onUndo?.();
                onDismiss(t.id);
              }}
            >
              {t.undoLabel ?? "Undo"}
            </button>
          )}
          <button
            className="toast-close"
            onClick={() => onDismiss(t.id)}
            aria-label="Dismiss"
            title="Dismiss"
          >
            <Icon name="x" size={14} />
          </button>
        </div>
      ))}
    </div>
  );
}
