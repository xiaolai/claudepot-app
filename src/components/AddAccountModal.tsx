import { useEffect, useId, useState } from "react";
import { api } from "../api";
import { useFocusTrap } from "../hooks/useFocusTrap";

export function AddAccountModal({
  onClose,
  onAdded,
  onError,
}: {
  onClose: () => void;
  onAdded: () => void;
  onError: (msg: string) => void;
}) {
  const [busy, setBusy] = useState(false);
  const trapRef = useFocusTrap<HTMLDivElement>();
  const titleId = useId();

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const submit = async () => {
    setBusy(true);
    try {
      await api.accountAddFromCurrent();
      onAdded();
    } catch (e) {
      onError(`add failed: ${e}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div ref={trapRef} className="modal" role="dialog" aria-modal="true"
        aria-labelledby={titleId} onClick={(e) => e.stopPropagation()}>
        <h2 id={titleId}>Add account</h2>
        <div className="modal-body">
          <p className="muted">
            Imports whichever account Claude Code is currently signed into.
            Log in with <code>claude auth login</code> first if needed.
          </p>
          <p className="muted small">
            For headless or token-based onboarding, use the{" "}
            <code>claudepot</code> CLI &mdash; refresh tokens never enter the
            GUI to avoid leaking secrets through the webview.
          </p>
        </div>
        <div className="modal-actions">
          <button onClick={onClose} disabled={busy}>Cancel</button>
          <button className="primary" onClick={submit} disabled={busy}>
            {busy ? "Adding…" : "Add from current"}
          </button>
        </div>
      </div>
    </div>
  );
}
