import { ConfirmDialog } from "./ConfirmDialog";
import { SplitBrainConfirm } from "../sections/accounts/SplitBrainConfirm";
import { DesktopConfirmDialog } from "../sections/accounts/DesktopConfirmDialog";
import { useAppState } from "../providers/AppStateProvider";

/**
 * Shell-level account confirmation modals, extracted from AppShell.
 * Reads its pending-request state straight from AppStateProvider —
 * no prop threading. Renders nothing until one of the three flows
 * (split-brain CLI swap, Desktop overwrite/sign-out, account remove)
 * parks a pending request.
 */
export function AccountConfirmModals() {
  const {
    splitBrainPending,
    dismissSplitBrain,
    confirmSplitBrain,
    desktopConfirmPending,
    dismissDesktopConfirm,
    confirmDesktopPending,
    removeConfirmPending,
    dismissRemoveConfirm,
    confirmRemoveAccount,
  } = useAppState();

  return (
    <>
      {splitBrainPending && (
        <SplitBrainConfirm
          account={splitBrainPending}
          onCancel={dismissSplitBrain}
          onConfirm={confirmSplitBrain}
        />
      )}

      {desktopConfirmPending && (
        <DesktopConfirmDialog
          request={desktopConfirmPending}
          onCancel={dismissDesktopConfirm}
          onConfirm={confirmDesktopPending}
        />
      )}

      {removeConfirmPending && (
        <ConfirmDialog
          title="Remove account?"
          confirmLabel="Remove"
          confirmDanger
          body={
            <>
              <p>
                Remove <strong>{removeConfirmPending.email}</strong>?
              </p>
              <p className="muted small">
                Deletes credentials and Desktop profile. Active
                CLI/Desktop pointers will be cleared. You'll have a few
                seconds to undo from the toast.
              </p>
            </>
          }
          onCancel={dismissRemoveConfirm}
          onConfirm={confirmRemoveAccount}
        />
      )}
    </>
  );
}
