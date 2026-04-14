import React, { useEffect } from "react";
import { useFocusTrap } from "../hooks/useFocusTrap";

export function ConfirmDialog({
  title,
  body,
  confirmLabel = "Confirm",
  confirmDanger = false,
  onCancel,
  onConfirm,
}: {
  title: string;
  body: React.ReactNode;
  confirmLabel?: string;
  confirmDanger?: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const trapRef = useFocusTrap<HTMLDivElement>();

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onCancel();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onCancel]);

  return (
    <div className="modal-backdrop" onClick={onCancel}>
      <div ref={trapRef} className="modal" role="dialog" aria-modal="true"
        aria-labelledby="confirm-title" onClick={(e) => e.stopPropagation()}>
        <h2 id="confirm-title">{title}</h2>
        <div className="modal-body">{body}</div>
        <div className="modal-actions">
          <button onClick={onCancel}>Cancel</button>
          <button className={confirmDanger ? "danger primary" : "primary"}
            onClick={onConfirm} autoFocus>{confirmLabel}</button>
        </div>
      </div>
    </div>
  );
}
