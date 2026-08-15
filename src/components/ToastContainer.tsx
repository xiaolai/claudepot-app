import { useTranslation } from "react-i18next";
import type { Toast } from "../hooks/useToasts";
import { Glyph } from "../components/primitives/Glyph";
import { NF } from "../icons";

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
  /**
   * Dismiss commits the toast's deferred action by default (X-close
   * means "I read it, proceed"); only the Undo button passes
   * `skipCommit` — undoing and committing are mutually exclusive.
   */
  onDismiss: (id: number, opts?: { skipCommit?: boolean }) => void;
}) {
  const { t: tr } = useTranslation("components");
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
                onDismiss(t.id, { skipCommit: true });
              }}
            >
              {t.undoLabel ?? tr("toasts.undo")}
            </button>
          )}
          <button
            className="toast-close"
            onClick={() => onDismiss(t.id)}
            aria-label={tr("toasts.dismiss")}
            title={tr("toasts.dismiss")}
          >
            <Glyph g={NF.x} style={{ fontSize: 14 }} />
          </button>
        </div>
      ))}
    </div>
  );
}
