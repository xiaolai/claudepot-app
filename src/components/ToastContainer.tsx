import { X } from "lucide-react";
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
              Undo
            </button>
          )}
          <button
            className="toast-close"
            onClick={() => onDismiss(t.id)}
            aria-label="Dismiss"
            title="Dismiss"
          >
            <X size={14} strokeWidth={2.5} />
          </button>
        </div>
      ))}
    </div>
  );
}
