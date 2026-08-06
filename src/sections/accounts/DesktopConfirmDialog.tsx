import { Trans, useTranslation } from "react-i18next";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import type { DesktopConfirmRequest } from "../../providers/AppStateProvider";

interface Props {
  request: DesktopConfirmRequest;
  onCancel: () => void;
  onConfirm: () => void;
}

/**
 * Destructive-Desktop-action confirmations. Codex follow-up review
 * (thread 019db814-a45b-7fa3-a280-80b1f20e1149) flagged that
 * "Sign Desktop out" and adopt-overwrite were one-click destructive
 * with no confirmation. The same copy runs through the tray, context
 * menu, and palette surfaces so the user sees the same trade-off
 * regardless of entry point.
 */
export function DesktopConfirmDialog({ request, onCancel, onConfirm }: Props) {
  const { t } = useTranslation("accounts");
  switch (request.kind) {
    case "sign_out":
      return (
        <ConfirmDialog
          title={t("desktopConfirm.signOut.title")}
          confirmLabel={t("desktopConfirm.signOut.confirm")}
          confirmDanger
          body={
            <>
              <p>{t("desktopConfirm.signOut.body")}</p>
              <ul className="muted small" style={{ paddingLeft: 18 }}>
                <li>{t("desktopConfirm.signOut.bullet1")}</li>
                <li>{t("desktopConfirm.signOut.bullet2")}</li>
                <li>
                  <Trans
                    i18nKey="desktopConfirm.signOut.bullet3"
                    ns="accounts"
                    values={{ path: "~/.claudepot/desktop/" }}
                    components={{ c: <code /> }}
                  />
                </li>
              </ul>
            </>
          }
          onCancel={onCancel}
          onConfirm={onConfirm}
        />
      );
    case "overwrite_profile":
      return (
        <ConfirmDialog
          title={t("desktopConfirm.overwrite.title", {
            email: request.account.email,
          })}
          confirmLabel={t("desktopConfirm.overwrite.confirm")}
          confirmDanger
          body={
            <>
              <p>
                <Trans
                  i18nKey="desktopConfirm.overwrite.body"
                  ns="accounts"
                  values={{ email: request.account.email }}
                  components={{ emph: <strong /> }}
                />
              </p>
              <p className="muted small">
                {t("desktopConfirm.overwrite.note")}
              </p>
            </>
          }
          onCancel={onCancel}
          onConfirm={onConfirm}
        />
      );
  }
}
