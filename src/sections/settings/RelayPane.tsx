/**
 * Settings → Relay — serve this Mac's Claude subscription to Claude Code
 * on other machines you pair.
 *
 * ## What this is not
 *
 * Not Settings → Remote. That one is a control panel behind an admin
 * password, and behind it is code execution as this user. This one
 * answers inference requests behind per-device tokens and reaches
 * nothing else. The two share TLS plumbing and nothing the user sees.
 *
 * ## Two liveness fields, as in Remote
 *
 * `serving` is the machine (the serve lock is held); `runningHere` is
 * this process. They differ when a `claudepot relay serve` runs in a
 * terminal — and then Stop here cannot touch it, and every change below
 * is refused until it stops, so the pane says so instead of offering
 * controls that will fail.
 *
 * ## Why changes need the relay stopped
 *
 * Publications, bind and devices are files the serving process read at
 * start. Revoke is the exception: it always goes through a live relay
 * when one serves, so a revoked device's streams end at once — this
 * process's own, or a terminal relay's revoke socket. Only a terminal
 * relay without a socket (Windows, or one that could not bind) refuses
 * it. A revoke the terminal relay took but never confirmed is
 * `relay.revoke_outcome_unknown`: maybe revoked, and safe to repeat.
 *
 * ## Layout
 *
 * State, polling and verbs are `useRelayPaneController`; each section is
 * its own component in `RelayPane.{server,publishing,devices}.tsx`.
 */
import { useTranslation } from "react-i18next";

import { relayApi } from "../../api";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import { Glyph } from "../../components/primitives/Glyph";
import { SkeletonList } from "../../components/primitives/Skeleton";
import { NF } from "../../icons";
import { useRelayPaneController } from "./useRelayPaneController";
import { RelayAddressSection, RelayStatusSection } from "./RelayPane.server";
import { RelayAccountsSection, RelayPublicationsSection } from "./RelayPane.publishing";
import { RelayDevicesSection, RelayPairSection } from "./RelayPane.devices";
import { DANGER, FAINT, NOTE, PANE, WARN } from "./RelayPane.styles";

type Props = {
  pushToast?: (kind: "info" | "error", text: string) => void;
};

export function RelayPane({ pushToast }: Props) {
  const { t } = useTranslation("settings");
  const c = useRelayPaneController(pushToast);
  const { status, loadError, confirmRevoke } = c;

  if (!status) {
    if (loadError) {
      return (
        <div style={DANGER} role="alert">
          {loadError}
        </div>
      );
    }
    return <SkeletonList rows={4} />;
  }

  return (
    <div style={PANE}>
      {/* A failed poll after a good one: say so, and keep showing the
          last state read rather than blanking the pane. */}
      {loadError && (
        <div style={{ ...DANGER, marginTop: 0 }} role="alert">
          {loadError}
        </div>
      )}
      <div style={{ ...NOTE, marginTop: 0 }}>{t("relay.intro")}</div>
      <div style={FAINT}>{t("relay.boundary")}</div>
      <div style={FAINT}>{t("relay.terms")}</div>

      {status.configRecovered && (
        <div style={WARN} role="alert">
          <Glyph g={NF.warn} /> {t("relay.configRecovered")}
        </div>
      )}
      {status.devicesRecovered && (
        <div style={WARN} role="alert">
          <Glyph g={NF.warn} /> {t("relay.devicesRecovered")}
        </div>
      )}

      <RelayStatusSection c={c} status={status} />
      <RelayAccountsSection c={c} status={status} />
      <RelayPublicationsSection c={c} status={status} />
      <RelayAddressSection c={c} status={status} />
      <RelayDevicesSection c={c} status={status} />
      <RelayPairSection c={c} status={status} />

      {confirmRevoke && (
        <ConfirmDialog
          title={t("relay.revokeTitle", { name: confirmRevoke.name })}
          body={t("relay.revokeBody")}
          confirmLabel={t("relay.revoke")}
          confirmDanger
          onCancel={() => c.setConfirmRevoke(null)}
          onConfirm={() => {
            const d = confirmRevoke;
            c.setConfirmRevoke(null);
            void c.run(
              `revoke-${d.id}`,
              () => relayApi.relayRevokeDevice(d.id),
              (r) => (r.alreadyRevoked ? t("relay.alreadyRevoked") : t("relay.revokedOk")),
              t("relay.revokeScope"),
            );
          }}
        />
      )}
    </div>
  );
}
