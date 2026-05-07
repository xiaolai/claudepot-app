import { useTranslation } from "react-i18next";
import { Icon } from "./Icon";

export function EmptyState({ onAdd }: { onAdd: () => void }) {
  const { t } = useTranslation();
  return (
    <div className="empty onboarding">
      <Icon name="user-plus" size={32} />
      <h2>{t("emptyState.title")}</h2>

      <div className="onboarding-steps">
        <div className="onboarding-step">
          <span className="onboarding-step-number">1</span>
          <div>
            <p className="onboarding-step-title">{t("emptyState.step1Title")}</p>
            <p className="muted onboarding-step-detail">
              <Icon name="terminal" size={12} />{" "}
              <code>claude auth login</code>
            </p>
          </div>
        </div>
        <div className="onboarding-step">
          <span className="onboarding-step-number">2</span>
          <div>
            <p className="onboarding-step-title">{t("emptyState.step2Title")}</p>
            <p className="muted onboarding-step-detail">
              {t("emptyState.step2Detail")}
            </p>
          </div>
        </div>
      </div>

      <button className="btn primary" onClick={onAdd}>
        {t("emptyState.addButton")}
      </button>
      <p className="muted onboarding-repeat-hint">
        {t("emptyState.repeatHint")}
      </p>
    </div>
  );
}
