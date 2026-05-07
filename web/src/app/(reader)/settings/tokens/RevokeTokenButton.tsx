"use client";

import { useActionState, useEffect, useState } from "react";

import {
  revokeApiTokenFormAction,
  type RevokeFormState,
} from "@/lib/actions/api-tokens";
import { ConfirmDialog } from "@/components/prototype/ConfirmDialog";

const INIT: RevokeFormState = { phase: "idle" };

export function RevokeTokenButton({
  tokenId,
  tokenName,
}: {
  tokenId: string;
  tokenName: string;
}) {
  const [state, formAction, pending] = useActionState(
    revokeApiTokenFormAction,
    INIT,
  );
  const [open, setOpen] = useState(false);

  // Auto-close the dialog once the action settles in either direction.
  // Errors stay visible inline beside the trigger button so the user
  // can re-open and retry.
  useEffect(() => {
    if (!pending && (state.phase === "ok" || state.phase === "err")) {
      setOpen(false);
    }
  }, [pending, state.phase]);

  function handleConfirm() {
    const fd = new FormData();
    fd.set("tokenId", tokenId);
    formAction(fd);
  }

  return (
    <div className="proto-form-inline">
      <button
        type="button"
        className="proto-mod-btn proto-mod-btn-remove"
        disabled={pending}
        onClick={() => setOpen(true)}
      >
        {pending ? "Revoking…" : "Revoke"}
      </button>
      {state.phase === "err" ? (
        <span className="proto-form-flash proto-form-flash-err">
          {state.message}
        </span>
      ) : null}
      <ConfirmDialog
        open={open}
        pending={pending}
        onClose={() => setOpen(false)}
        onConfirm={handleConfirm}
        title={`Revoke "${tokenName}"?`}
        description={
          <p>
            Any clients still using this token will start getting{" "}
            <strong>401 errors</strong> immediately. This cannot be undone.
          </p>
        }
        confirmLabel="Revoke token"
        pendingLabel="Revoking…"
        variant="danger"
      />
    </div>
  );
}
