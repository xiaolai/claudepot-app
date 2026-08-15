import { Trans, useTranslation } from "react-i18next";
import { Glyph } from "../../components/primitives/Glyph";
import { NF } from "../../icons";
import { i18n } from "../../lib/i18n";
import type { AccountSummary } from "../../types";
import { requiresAttention } from "./verifyStatus";

interface AnomalyBannerProps {
  account: AccountSummary;
  onRelogin?: () => void;
  disabled?: boolean;
}

/**
 * In-card warning strip. Triggers on drift / rejected / signed-out /
 * unhealthy —
 * see `isAnomaly(a)`. Always exposes a "Re-login" button that fires
 * `onRelogin` (wired to `api.accountLogin`).
 *
 * `token_status === "expired"` is intentionally NOT an anomaly: the
 * verify pass auto-refreshes via the OAuth refresh_token within a
 * second of the next focus/refresh tick, so a banner would just
 * flash and disappear. If the refresh itself fails, verify_status
 * flips to "rejected" and that branch carries the actionable signal.
 */
export function AnomalyBanner({
  account,
  onRelogin,
  disabled,
}: AnomalyBannerProps) {
  const { t } = useTranslation("accounts");
  const copy = anomalyCopy(account);
  if (!copy) return null;

  return (
    <div
      role="alert"
      style={{
        padding: "var(--sp-10) var(--sp-18)",
        background: "color-mix(in oklch, var(--warn) 12%, transparent)",
        borderBottom: "var(--bw-hair) solid var(--line)",
        display: "flex",
        alignItems: "flex-start",
        gap: "var(--sp-10)",
      }}
    >
      <Glyph
        g={NF.warn}
        color="var(--warn)"
        style={{ fontSize: "var(--fs-base)", marginTop: "var(--sp-2)" }}
      />
      <div
        style={{
          flex: 1,
          minWidth: 0,
          fontSize: "var(--fs-xs)",
        }}
      >
        <div
          style={{
            fontWeight: 600,
            color: "var(--fg)",
            marginBottom: "var(--sp-2)",
          }}
        >
          {copy.title}
        </div>
        <div style={{ color: "var(--fg-muted)" }}>{copy.detail}</div>
      </div>
      <button
        type="button"
        onClick={onRelogin}
        disabled={disabled}
        title={t("anomaly.reloginTitle")}
        style={{
          padding: "var(--sp-3) var(--sp-8)",
          fontSize: "var(--fs-xs)",
          background: "var(--bg-raised)",
          border: "var(--bw-hair) solid var(--line-strong)",
          borderRadius: "var(--r-1)",
          color: "var(--fg)",
          whiteSpace: "nowrap",
          fontWeight: 500,
          opacity: disabled ? "var(--opacity-dimmed)" : 1,
        }}
      >
        {t("anomaly.relogin")}
      </button>
    </div>
  );
}

/**
 * True when the account is in a state that needs human attention.
 * The four cases match `anomalyCopy` below one-to-one. Locally
 * `expired` tokens are excluded — the verify pass refreshes them
 * automatically; if the refresh fails the row flips to `rejected`.
 *
 * `signed_out` had to be added here as well as to the copy: the banner
 * is the only surface carrying the Re-login button, so a terminal state
 * that fails this predicate is one the user cannot act on from the card
 * at all. That was the visible half of issue #74 — an unverified chip,
 * no banner, and nothing to click.
 */
export function isAnomaly(a: AccountSummary): boolean {
  // `a.drift` is kept alongside the status check because the DTO
  // computes it separately (`verify_status === "drift"` backend-side);
  // trusting only one of the two would make this predicate depend on
  // which field happened to be refreshed last.
  return a.drift || requiresAttention(a);
}

function anomalyCopy(
  a: AccountSummary,
): { title: string; detail: React.ReactNode } | null {
  if (a.drift) {
    return {
      title: i18n.t("anomaly.drift.title", { ns: "accounts" }),
      detail: (
        <Trans
          i18nKey="anomaly.drift.detail"
          ns="accounts"
          values={{
            email:
              a.verified_email ||
              i18n.t("anomaly.drift.anotherAccount", { ns: "accounts" }),
          }}
          components={{ emph: <strong style={{ color: "var(--fg)" }} /> }}
        />
      ),
    };
  }
  if (a.verify_status === "rejected") {
    return {
      title: i18n.t("anomaly.rejected.title", { ns: "accounts" }),
      detail: i18n.t("anomaly.rejected.detail", { ns: "accounts" }),
    };
  }
  // Deliberately worded away from "rejected": nothing refused this
  // login, because with empty tokens no request was ever made. Copy
  // that blames the server or the account sends the user to check
  // something that is fine.
  if (a.verify_status === "signed_out") {
    return {
      title: i18n.t("anomaly.signedOut.title", { ns: "accounts" }),
      detail: i18n.t("anomaly.signedOut.detail", { ns: "accounts" }),
    };
  }
  if (!a.credentials_healthy) {
    return {
      title: i18n.t("anomaly.broken.title", { ns: "accounts" }),
      detail: i18n.t("anomaly.broken.detail", { ns: "accounts" }),
    };
  }
  return null;
}
