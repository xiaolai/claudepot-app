import { Trans, useTranslation } from "react-i18next";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import type { AccountSummary } from "../../types";

interface Props {
  account: AccountSummary;
  onCancel: () => void;
  onConfirm: () => void;
}

/**
 * Pre-swap split-brain warning. Surfaces the same three-bullet
 * advisory the CLI prints *after* a `--force` swap when CC is
 * running — except the GUI raises it *before* the swap so the user
 * makes the trade-off knowingly rather than recovering from it.
 */
export function SplitBrainConfirm({ account, onCancel, onConfirm }: Props) {
  const { t } = useTranslation("accounts");
  return (
    <ConfirmDialog
      title={t("splitBrain.title")}
      confirmLabel={t("splitBrain.confirm", { email: account.email })}
      confirmDanger
      body={
        <>
          <p>{t("splitBrain.intro")}</p>
          <ul className="muted small" style={{ paddingLeft: 18 }}>
            <li>{t("splitBrain.bullet1")}</li>
            <li>
              <Trans
                i18nKey="splitBrain.bullet2"
                ns="accounts"
                values={{ email: account.email }}
                components={{ emph: <strong /> }}
              />
            </li>
            <li>{t("splitBrain.bullet3")}</li>
          </ul>
          <p className="muted small">
            <Trans
              i18nKey="splitBrain.note"
              ns="accounts"
              values={{ flag: "--force" }}
              components={{ c: <code /> }}
            />
          </p>
        </>
      }
      onCancel={onCancel}
      onConfirm={onConfirm}
    />
  );
}
