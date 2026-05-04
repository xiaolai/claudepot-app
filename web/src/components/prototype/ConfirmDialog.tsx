"use client";

import { useEffect, useId, useRef } from "react";
import type { ReactNode } from "react";
import { AlertTriangle } from "lucide-react";

interface Props {
  open: boolean;
  /** When true, buttons disable and dismissals (ESC, backdrop, cancel) are blocked. */
  pending?: boolean;
  onClose: () => void;
  onConfirm: () => void;
  title: string;
  description?: ReactNode;
  confirmLabel?: string;
  /** Shown on the confirm button while `pending`. Defaults to confirmLabel + "…". */
  pendingLabel?: string;
  cancelLabel?: string;
  /** "danger" tints the confirm button red and adds an icon to the title. */
  variant?: "default" | "danger";
}

/**
 * Modal confirmation dialog built on the native HTML <dialog> element
 * via showModal(). The browser handles focus trap, top-layer
 * rendering, ESC key, and focus return — we only style and dispatch.
 *
 * Three ways out: Cancel button, ESC, click outside the body. All
 * three call onClose, so the parent stays in sync. While `pending`,
 * all three are blocked — the parent must drive close after the
 * underlying action settles.
 */
export function ConfirmDialog({
  open,
  pending = false,
  onClose,
  onConfirm,
  title,
  description,
  confirmLabel = "Confirm",
  pendingLabel,
  cancelLabel = "Cancel",
  variant = "default",
}: Props) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  const titleId = useId();

  // Sync `open` prop → native dialog state. showModal() is what
  // promotes the <dialog> to the top layer and gives us the focus
  // trap; calling .open = true imperatively would NOT do that.
  useEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog) return;
    if (open && !dialog.open) {
      dialog.showModal();
    } else if (!open && dialog.open) {
      dialog.close();
    }
  }, [open]);

  // The dialog dispatches "close" for any dismissal (ESC, .close()
  // call, form-submit-with-method-dialog). Treat all of them as a
  // cancel signal so the parent's `open` state can't drift out of
  // sync with the actual visibility.
  useEffect(() => {
    const dialog = dialogRef.current;
    if (!dialog) return;
    const handler = () => onClose();
    dialog.addEventListener("close", handler);
    return () => dialog.removeEventListener("close", handler);
  }, [onClose]);

  // Block ESC dismissal while pending. The "cancel" event fires
  // before "close" and is preventable per the HTML spec.
  useEffect(() => {
    if (!pending) return;
    const dialog = dialogRef.current;
    if (!dialog) return;
    const handler = (e: Event) => e.preventDefault();
    dialog.addEventListener("cancel", handler);
    return () => dialog.removeEventListener("cancel", handler);
  }, [pending]);

  // Backdrop click → close. The native <dialog> doesn't fire a
  // dedicated event for this; clicks on the backdrop bubble up to
  // the dialog itself, so we check `e.target === dialog` to
  // distinguish backdrop from body content.
  function handleBackdropClick(e: React.MouseEvent<HTMLDialogElement>) {
    if (pending) return;
    if (e.target === dialogRef.current) onClose();
  }

  const liveConfirmLabel = pending
    ? (pendingLabel ?? `${confirmLabel}…`)
    : confirmLabel;

  return (
    <dialog
      ref={dialogRef}
      className={`proto-confirm-dialog proto-confirm-dialog-${variant}`}
      onClick={handleBackdropClick}
      aria-labelledby={titleId}
    >
      <div className="proto-confirm-body">
        <h2 id={titleId} className="proto-confirm-title">
          {variant === "danger" && (
            <span className="proto-inline-icon" aria-hidden>
              <AlertTriangle size={18} />
            </span>
          )}
          {title}
        </h2>
        {description ? (
          <div className="proto-confirm-description">{description}</div>
        ) : null}
        <div className="proto-confirm-actions">
          <button
            type="button"
            className="proto-confirm-btn proto-confirm-btn-cancel"
            onClick={onClose}
            disabled={pending}
            autoFocus
          >
            {cancelLabel}
          </button>
          <button
            type="button"
            className={`proto-confirm-btn proto-confirm-btn-confirm proto-confirm-btn-confirm-${variant}`}
            onClick={onConfirm}
            disabled={pending}
          >
            {liveConfirmLabel}
          </button>
        </div>
      </div>
    </dialog>
  );
}
