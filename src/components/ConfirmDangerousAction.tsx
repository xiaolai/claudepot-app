import { useEffect, useRef, useState } from "react";
import { useFocusTrap } from "../hooks/useFocusTrap";

/**
 * Consequence-explaining confirm dialog. Two variants:
 *
 * - Standard (default): renders consequence copy + Cancel/Confirm
 *   buttons. Used for Resume, Rollback, Break-lock — all reversible
 *   or auditable.
 * - Type-to-confirm (`typeToConfirm` prop): renders an input field
 *   that must match the expected string before the Confirm button
 *   enables. Used for Abandon, which destroys the audit trail.
 *
 * All modals follow accessibility.md: role=dialog + aria-modal,
 * Escape closes, focus trap via tabindex on the modal, backdrop click
 * closes. (Backdrop click intentionally skipped here because
 * destructive actions should require an explicit Cancel click.)
 */
export function ConfirmDangerousAction({
  title,
  consequences,
  confirmLabel,
  onCancel,
  onConfirm,
  typeToConfirm,
  danger = true,
}: {
  title: string;
  /** React node so callers can render lists, code spans, etc. */
  consequences: React.ReactNode;
  confirmLabel: string;
  onCancel: () => void;
  onConfirm: () => void;
  /** When set, user must type this exact string before Confirm enables. */
  typeToConfirm?: string;
  /** Render Confirm in danger style. Defaults to true. */
  danger?: boolean;
}) {
  const [typed, setTyped] = useState("");
  const headingId = useRef(
    `cda-heading-${Math.random().toString(36).slice(2, 9)}`,
  );
  const trapRef = useFocusTrap<HTMLDivElement>();
  const confirmDisabled =
    typeToConfirm !== undefined && typed !== typeToConfirm;

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onCancel();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onCancel]);

  return (
    <div className="modal-backdrop" role="presentation">
      <div
        ref={trapRef}
        className="modal"
        role="dialog"
        aria-modal="true"
        aria-labelledby={headingId.current}
      >
        <h2 id={headingId.current}>{title}</h2>
        <div className="modal-body">{consequences}</div>
        {typeToConfirm !== undefined && (
          <div className="type-to-confirm">
            <label className="detail-label" htmlFor="type-to-confirm-input">
              Type <code className="mono">{typeToConfirm}</code> to confirm
            </label>
            <input
              id="type-to-confirm-input"
              type="text"
              autoComplete="off"
              autoCapitalize="off"
              spellCheck={false}
              value={typed}
              onChange={(e) => setTyped(e.target.value)}
              autoFocus
            />
          </div>
        )}
        <div className="modal-actions">
          <button type="button" onClick={onCancel}>
            Cancel
          </button>
          <button
            type="button"
            className={danger ? "danger primary" : "primary"}
            disabled={confirmDisabled}
            onClick={onConfirm}
            autoFocus={typeToConfirm === undefined}
          >
            {confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}
