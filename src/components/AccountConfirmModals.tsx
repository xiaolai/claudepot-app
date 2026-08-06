import { Trans, useTranslation } from "react-i18next";
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
  const { t } = useTranslation("components");
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
          title={t("modals.removeAccountTitle")}
          confirmLabel={t("modals.removeAccountConfirm")}
          confirmDanger
          body={
            <>
              <p>
                {/* Genuine mid-sentence element — the email is bolded
                    inside the question. */}
                <Trans
                  ns="components"
                  i18nKey="modals.removeAccountBody"
                  values={{ email: removeConfirmPending.email }}
                  components={{ b: <strong /> }}
                />
              </p>
              <p className="muted small">{t("modals.removeAccountNote")}</p>
            </>
          }
          onCancel={dismissRemoveConfirm}
          onConfirm={confirmRemoveAccount}
        />
      )}
    </>
  );
}
