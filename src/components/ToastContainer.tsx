import { Icon } from "./Icon";
import type { Toast } from "../hooks/useToasts";

/**
 * Toast queue renderer. Each toast carries its OWN ARIA role rather
 * than inheriting one from the container — `role="alert"` on errors
 * makes screen readers interrupt to announce, while `role="status"`
 * on info stays polite. The deleted `Toast` primitive used the same
 * split; preserving it after consolidation matches the design rules'
 * accessibility floor.
 *
 * No `aria-live` on the wrapper: the per-toast role is the live
 * region. A wrapping live region would announce every toast politely
 * regardless of kind, defeating the assertive announcement we want
 * for errors.
 */
export function ToastContainer({
  toasts,
  onDismiss,
}: {
  toasts: Toast[];
  onDismiss: (id: number) => void;
}) {
  return (
    <div className="toasts">
      {toasts.map((t) => (
        <div
          key={t.id}
          className={`toast ${t.kind} ${t.exiting ? "exiting" : ""}`}
          role={t.kind === "error" ? "alert" : "status"}
          aria-live={t.kind === "error" ? "assertive" : "polite"}
        >
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
