// Settings → Relay: the Paired devices and Pair a machine sections.
import { useTranslation } from "react-i18next";

import type { RelayDayUsage, RelayDevice, RelayStatus } from "../../api";
import { Button } from "../../components/primitives/Button";
import { EmptyState } from "../../components/primitives/EmptyState";
import { SectionLabel } from "../../components/primitives/SectionLabel";
import type { RelayPaneController } from "./useRelayPaneController";
import { FAINT, NOTE, ROW_META, ROW_NAME, SECTION } from "./RelayPane.styles";

export function RelayDevicesSection({ c, status }: { c: RelayPaneController; status: RelayStatus }) {
  const { t } = useTranslation("settings");
  const liveDevices = status.devices.filter((d) => !d.revokedAt);
  // A relay serving from a terminal holds the serve lock, so revoking
  // here goes through its revoke socket. Without one (Windows, or it
  // could not bind) there is no way in until it stops.
  const unreachable = status.serving && !status.runningHere && !status.revokeSocketPresent;
  // Serving here: Revoke in this window always works, but a terminal
  // `relay revoke` cannot reach this relay. On Windows that is the
  // platform, not a fault, so it is not worth a line.
  const noLiveRevoke =
    status.runningHere && status.liveRevokeUnavailable !== null && status.liveRevokeUnavailable !== "unsupported"
      ? status.liveRevokeUnavailable
      : null;
  return (
    <section style={SECTION}>
      <SectionLabel
        right={
          liveDevices.length > 0 ? (
            <span className="mono-cap" style={ROW_META}>
              {t("relay.deviceCount", { count: liveDevices.length })}
            </span>
          ) : undefined
        }
      >
        {t("relay.devicesHeading")}
      </SectionLabel>
      {liveDevices.length === 0 ? (
        // The next step — pairing — is the section directly below.
        <EmptyState variant="inline" body={t("relay.noDevices")} action={null} />
      ) : (
        <ul className="remote-devices">
          {liveDevices.map((d) => (
            <DeviceRow
              key={d.id}
              d={d}
              disabled={c.busy !== null || status.devicesRecovered || unreachable}
              onRevoke={() => c.setConfirmRevoke(d)}
            />
          ))}
        </ul>
      )}
      {status.usageUnavailable && <div style={FAINT}>{t("relay.usageUnavailable")}</div>}
      {status.devicesRecovered && <div style={FAINT}>{t("relay.cannotRevokeRecovered")}</div>}
      {!status.devicesRecovered && unreachable && liveDevices.length > 0 && (
        <div style={FAINT}>{t("relay.cannotRevokeElsewhere")}</div>
      )}
      {noLiveRevoke && liveDevices.length > 0 && (
        <div style={FAINT}>
          {t("relay.noLiveRevoke", { reason: t(`relay.noLiveRevokeReason.${noLiveRevoke}`, { defaultValue: noLiveRevoke }) })}
        </div>
      )}
    </section>
  );
}

export function RelayPairSection({ c, status }: { c: RelayPaneController; status: RelayStatus }) {
  const { t } = useTranslation("settings");
  const { offer } = c;
  const expired = offer !== null && c.offerRemainingMs <= 0;
  return (
    <section style={SECTION}>
      <SectionLabel>{t("relay.pairHeading")}</SectionLabel>
      <div style={FAINT}>{t("relay.pairWhy")}</div>
      <div className="remote-toggle">
        <input
          id="relay-include-future"
          type="checkbox"
          checked={c.includeFuture}
          disabled={c.busy !== null}
          aria-describedby="relay-include-future-detail"
          onChange={(e) => c.setIncludeFuture(e.target.checked)}
          style={{ marginTop: "var(--sp-3)", flexShrink: 0, accentColor: "var(--accent)" }}
        />
        <span>
          <label htmlFor="relay-include-future" style={{ fontSize: "var(--fs-sm)", color: "var(--fg)" }}>
            {t("relay.includeFutureLabel")}
          </label>
          <span id="relay-include-future-detail" style={{ fontSize: "var(--fs-xs)", color: "var(--fg-faint)" }}>
            {t("relay.includeFutureDetail")}
          </span>
        </span>
      </div>
      <div className="remote-actions">
        <Button variant="ghost" disabled={c.busy !== null || !status.runningHere} onClick={() => void c.makePairCode()}>
          {t("relay.pair")}
        </Button>
        {!status.runningHere && <span style={FAINT}>{t("relay.cannotPairNotServing")}</span>}
      </div>
      {offer && expired && (
        <div style={FAINT} role="status">
          {t("relay.pairExpired")}
        </div>
      )}
      {offer && !expired && (
        <div style={NOTE} role="status">
          <div className="mono selectable" style={{ fontSize: "var(--fs-md)", color: "var(--fg)" }}>
            {offer.code}
          </div>
          <div>{t("relay.pairExpires", { count: Math.max(1, Math.ceil(c.offerRemainingMs / 60_000)) })}</div>
          <div style={FAINT}>
            {t("relay.pairFingerprint")} <span className="mono selectable">{offer.caFingerprint}</span>
          </div>
        </div>
      )}
    </section>
  );
}

/** Today's usage, render-if-nonzero. Always an estimate (`≈`), and an
 *  unpriced request is counted, never priced: a mixed day states the
 *  priced subtotal against the priced count, not against every request. */
function useUsageText(u: RelayDayUsage): string | null {
  const { t } = useTranslation("settings");
  if (u.requests === 0) return null;
  if (u.unpriced === 0) return t("relay.today", { count: u.requests, cost: u.costUsd.toFixed(2) });
  if (u.unpriced >= u.requests) return t("relay.todayUnpriced", { count: u.requests });
  return t("relay.todayMixed", {
    count: u.requests,
    cost: u.costUsd.toFixed(2),
    priced: u.requests - u.unpriced,
    unpriced: u.unpriced,
  });
}

/** A paired device: grants, last seen, today's estimated spend. */
function DeviceRow({ d, disabled, onRevoke }: { d: RelayDevice; disabled: boolean; onRevoke: () => void }) {
  const { t } = useTranslation("settings");
  const usage = useUsageText(d.today);
  const parts: string[] = [];
  if (d.grants.length > 0) parts.push(t("relay.grants", { count: d.grants.length }));
  if (d.includeFuture) parts.push(t("relay.includesFuture"));
  if (d.lastSeen) parts.push(t("relay.lastSeen", { when: new Date(d.lastSeen).toLocaleString() }));
  if (usage) parts.push(usage);
  return (
    <li>
      <span style={ROW_NAME}>{d.name}</span>
      <span className="mono-cap" style={ROW_META}>
        {parts.join(" · ")}
      </span>
      <Button variant="ghost" danger disabled={disabled} onClick={onRevoke}>
        {t("relay.revoke")}
      </Button>
    </li>
  );
}
