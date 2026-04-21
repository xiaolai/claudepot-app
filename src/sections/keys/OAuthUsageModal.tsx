import { useEffect, useId, useState } from "react";
import { api } from "../../api";
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
  | { status: "ok"; usage: AccountUsage }
  | { status: "error"; detail: string };

/**
 * Mini usage modal that opens when the user clicks the account tag on
 * an OAuth token row. Reuses `/api/oauth/usage` — same endpoint the
 * Accounts section uses — with the row's stored token instead of the
 * account's cached access token.
 */
export function OAuthUsageModal({
  token,
  onClose,
}: {
  token: OauthTokenSummary;
  onClose: () => void;
}) {
  const [state, setState] = useState<State>({ status: "loading" });
  const titleId = useId();

  useEffect(() => {
    let cancelled = false;
    api
      .keyOauthUsage(token.uuid)
      .then((usage) => {
        if (!cancelled) setState({ status: "ok", usage });
      })
      .catch((e) => {
        if (!cancelled) setState({ status: "error", detail: `${e}` });
      });
    return () => {
      cancelled = true;
    };
  }, [token.uuid]);

  return (
    <Modal open onClose={onClose} width="md" aria-labelledby={titleId}>
      <ModalHeader
        glyph={NF.bolt}
        title={`Usage · ${token.label}`}
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
            <Tag tone="accent">
              {token.account_email ?? token.account_uuid.slice(0, 8)}
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
            <p style={{ margin: 0, color: "var(--fg-muted)" }}>Loading…</p>
          )}

          {state.status === "error" && (
            <p style={{ margin: 0, color: "var(--danger)" }}>{state.detail}</p>
          )}

          {state.status === "ok" && <UsageBody usage={state.usage} />}
        </div>
      </ModalBody>
      <ModalFooter>
        <Button onClick={onClose} variant="ghost">
          Close
        </Button>
      </ModalFooter>
    </Modal>
  );
}

function UsageBody({ usage }: { usage: AccountUsage }) {
  const rows: Array<[string, UsageWindow | null]> = [
    ["5-hour window", usage.five_hour],
    ["7-day window", usage.seven_day],
    ["7-day · Opus", usage.seven_day_opus],
    ["7-day · Sonnet", usage.seven_day_sonnet],
    ["7-day · OAuth apps", usage.seven_day_oauth_apps],
    ["7-day · Cowork", usage.seven_day_cowork],
  ];

  // Render-if-nonzero per design rules — drop rows that have no window.
  const visible = rows.filter(([, w]) => w !== null);
  const hasExtra = !!usage.extra_usage && usage.extra_usage.is_enabled;

  if (visible.length === 0 && !hasExtra) {
    return (
      <p style={{ margin: 0, color: "var(--fg-muted)" }}>
        No usage windows reported for this token yet.
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
      <span style={{ color: "var(--fg-muted)" }}>Extra usage</span>
      <span style={{ fontFeatureSettings: "'tnum'" }}>
        {pct.toFixed(1)}%
      </span>
    </div>
  );
}
