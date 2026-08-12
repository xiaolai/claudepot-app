import { useEffect, useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api";
import { renderError } from "../../lib/i18n-error";
import {
  Modal,
  ModalBody,
  ModalFooter,
  ModalHeader,
} from "../../components/primitives/Modal";
import { Button } from "../../components/primitives/Button";
import { Tag } from "../../components/primitives/Tag";
import { NF } from "../../icons";
import type { AccountUsage, OauthTokenSummary, UsageWindow } from "../../types";

type State =
  | { status: "loading" }
  | { status: "ok"; usage: AccountUsage | null }
  | { status: "error"; detail: string };

/**
 * Mini usage modal that opens when the user clicks the account tag on
 * an OAuth token row. Reads from the in-memory usage cache populated
 * by the Accounts section — no live Anthropic call is made here.
 * When the cache has no entry for the account, we show an empty hint
 * pointing the user at Accounts to refresh.
 */
export function OAuthUsageModal({
  token,
  onClose,
}: {
  token: OauthTokenSummary;
  onClose: () => void;
}) {
  const { t } = useTranslation("keys");
  const [state, setState] = useState<State>({ status: "loading" });
  const titleId = useId();

  useEffect(() => {
    let cancelled = false;
    api
      .keyOauthUsageCached(token.uuid)
      .then((usage) => {
        if (!cancelled) setState({ status: "ok", usage });
      })
      .catch((e) => {
        if (!cancelled) setState({ status: "error", detail: renderError(e) });
      });
    return () => {
      cancelled = true;
    };
  }, [token.uuid]);

  return (
    <Modal open onClose={onClose} width="md" aria-labelledby={titleId}>
      <ModalHeader
        glyph={NF.bolt}
        title={t("usageModal.title", { label: token.label })}
        id={titleId}
        onClose={onClose}
      />
      <ModalBody>
        <div
          style={{
            display: "flex",
            flexDirection: "column",
            gap: "var(--sp-10)",
          }}
        >
          <div
            style={{
              display: "flex",
              alignItems: "center",
              gap: "var(--sp-8)",
              flexWrap: "wrap",
            }}
          >
            <Tag tone={token.account_email ? "accent" : "warn"}>
              {token.account_email ?? t("list.accountRemoved")}
            </Tag>
            <code
              style={{
                fontSize: "var(--fs-xs)",
                color: "var(--fg-muted)",
              }}
            >
              {token.token_preview}
            </code>
          </div>

          {state.status === "loading" && (
            <p style={{ margin: 0, color: "var(--fg-muted)" }}>
              {t("usageModal.loading")}
            </p>
          )}

          {state.status === "error" && (
            <p style={{ margin: 0, color: "var(--danger)" }}>{state.detail}</p>
          )}

          {state.status === "ok" && state.usage === null && (
            <p style={{ margin: 0, color: "var(--fg-muted)" }}>
              {t("usageModal.noCached")}
            </p>
          )}

          {state.status === "ok" && state.usage !== null && (
            <UsageBody usage={state.usage} />
          )}
        </div>
      </ModalBody>
      <ModalFooter>
        <Button onClick={onClose} variant="ghost">
          {t("usageModal.close")}
        </Button>
      </ModalFooter>
    </Modal>
  );
}

export function UsageBody({ usage }: { usage: AccountUsage }) {
  const { t } = useTranslation("keys");
  // Render-time lookups, not a module constant — a language switch has
  // to reach these labels without a remount.
  const rows: Array<[string, UsageWindow | null]> = [
    [t("usageModal.fiveHour"), usage.five_hour],
    [t("usageModal.sevenDay"), usage.seven_day],
    [t("usageModal.sevenDayOpus"), usage.seven_day_opus],
    [t("usageModal.sevenDaySonnet"), usage.seven_day_sonnet],
    [t("usageModal.sevenDayOauthApps"), usage.seven_day_oauth_apps],
    [t("usageModal.sevenDayCowork"), usage.seven_day_cowork],
    // Model-scoped windows (e.g. the weekly Fable limit). The server
    // names the model, so the label interpolates rather than resolving
    // a fixed key. `?? []` guards a usage entry cached by an older
    // build, which has no such field.
    ...(usage.scoped_limits ?? []).map(
      (s) =>
        [
          t("usageModal.scopedWeekly", { model: s.label }),
          { utilization: s.utilization, resets_at: s.resets_at },
        ] as [string, UsageWindow | null],
    ),
  ];

  // Render-if-nonzero per design rules — drop rows that have no window.
  const visible = rows.filter(([, w]) => w !== null);
  const hasExtra = !!usage.extra_usage && usage.extra_usage.is_enabled;

  if (visible.length === 0 && !hasExtra) {
    return (
      <p style={{ margin: 0, color: "var(--fg-muted)" }}>
        {t("usageModal.noWindows")}
      </p>
    );
  }

  return (
    <div
      style={{
        display: "flex",
        flexDirection: "column",
        gap: "var(--sp-6)",
      }}
    >
      {visible.map(([label, window]) => (
        <Row key={label} label={label} window={window} />
      ))}
      {usage.extra_usage && usage.extra_usage.is_enabled && (
        <ExtraRow extra={usage.extra_usage} />
      )}
    </div>
  );
}

function Row({ label, window }: { label: string; window: UsageWindow | null }) {
  if (!window) return null;
  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        justifyContent: "space-between",
        padding: "var(--sp-6) var(--sp-8)",
        borderBottom: "var(--bw-hair) solid var(--line)",
        fontSize: "var(--fs-sm)",
      }}
    >
      <span style={{ color: "var(--fg-muted)" }}>{label}</span>
      <span style={{ fontFeatureSettings: "'tnum'" }}>
        {window.utilization.toFixed(1)}%
      </span>
    </div>
  );
}

function ExtraRow({
  extra,
}: {
  extra: NonNullable<AccountUsage["extra_usage"]>;
}) {
  const { t } = useTranslation("keys");
  const pct = extra.utilization ?? 0;
  return (
    <div
      style={{
        display: "flex",
        alignItems: "center",
        justifyContent: "space-between",
        padding: "var(--sp-6) var(--sp-8)",
        fontSize: "var(--fs-sm)",
      }}
    >
      <span style={{ color: "var(--fg-muted)" }}>
        {t("usageModal.extraUsage")}
      </span>
      <span style={{ fontFeatureSettings: "'tnum'" }}>
        {pct.toFixed(1)}%
      </span>
    </div>
  );
}
