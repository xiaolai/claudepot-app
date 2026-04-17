import { useEffect, useId, useState } from "react";
import { api } from "../api";
import { useFocusTrap } from "../hooks/useFocusTrap";

type Preflight =
  | { kind: "checking" }
  | { kind: "ready"; email: string }
  | { kind: "empty" } // CC has no blob
  | { kind: "error"; message: string };

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
  const [preflight, setPreflight] = useState<Preflight>({ kind: "checking" });
  const trapRef = useFocusTrap<HTMLDivElement>();
  const titleId = useId();

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  // Ground-truth check: what account is CC actually holding credentials
  // for right now? Without this, clicking "Add from current" while CC
  // isn't signed in produces a generic "add failed: …" with no hint of
  // the actual cause.
  useEffect(() => {
    let cancelled = false;
    api
      .currentCcIdentity()
      .then((identity) => {
        if (cancelled) return;
        if (identity.error) {
          setPreflight({ kind: "error", message: identity.error });
        } else if (identity.email) {
          setPreflight({ kind: "ready", email: identity.email });
        } else {
          setPreflight({ kind: "empty" });
        }
      })
      .catch((e) => {
        if (cancelled) return;
        setPreflight({ kind: "error", message: `${e}` });
      });
    return () => {
      cancelled = true;
    };
  }, []);

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

  const canAdd = preflight.kind === "ready";

  return (
    <div className="modal-backdrop" onClick={onClose}>
      <div ref={trapRef} className="modal" role="dialog" aria-modal="true"
        aria-labelledby={titleId} onClick={(e) => e.stopPropagation()}>
        <h2 id={titleId}>Add account</h2>
        <div className="modal-body">
          {preflight.kind === "checking" && (
            <p className="muted">Checking Claude Code…</p>
          )}
          {preflight.kind === "ready" && (
            <>
              <p>
                Claude Code is signed in as{" "}
                <strong className="selectable">{preflight.email}</strong>. Add
                this account?
              </p>
              <p className="muted small">
                For headless or token-based onboarding, use the{" "}
                <code>claudepot</code> CLI &mdash; refresh tokens never enter
                the GUI to avoid leaking secrets through the webview.
              </p>
            </>
          )}
          {preflight.kind === "empty" && (
            <>
              <p>
                Claude Code isn't signed in. Sign in first, then try again.
              </p>
              <pre className="mono small"><code>claude auth login</code></pre>
              <p className="muted small">
                After signing in, click <em>Retry</em> below — or close this
                dialog and use the sign-in button on an existing account card.
              </p>
            </>
          )}
          {preflight.kind === "error" && (
            <>
              <p>Couldn't read Claude Code credentials.</p>
              <p className="mono small">{preflight.message}</p>
            </>
          )}
        </div>
        <div className="modal-actions">
          <button onClick={onClose} disabled={busy}>Cancel</button>
          {preflight.kind === "ready" ? (
            <button className="primary" onClick={submit} disabled={busy}>
              {busy ? "Adding…" : "Add from current"}
            </button>
          ) : (
            <button
              className="primary"
              onClick={() => {
                setPreflight({ kind: "checking" });
                api
                  .currentCcIdentity()
                  .then((identity) => {
                    if (identity.error)
                      setPreflight({ kind: "error", message: identity.error });
                    else if (identity.email)
                      setPreflight({ kind: "ready", email: identity.email });
                    else setPreflight({ kind: "empty" });
                  })
                  .catch((e) =>
                    setPreflight({ kind: "error", message: `${e}` }),
                  );
              }}
              disabled={preflight.kind === "checking" || busy}
            >
              Retry
            </button>
          )}
        </div>
        {/* Keep the existing onAdd path reachable even when preflight says
            ready isn't met, in case the user genuinely wants to try — but
            disabled state above already suggests otherwise. */}
        {!canAdd && preflight.kind !== "checking" && (
          <p className="muted small submit-disclaimer">
            Add is disabled until Claude Code reports a signed-in account.
          </p>
        )}
      </div>
    </div>
  );
}
