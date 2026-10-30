import { useTranslation } from "react-i18next";

import { Glyph } from "./primitives/Glyph";
import { NF } from "../icons";
import { useRelayStatus } from "../hooks/useRelayStatus";

/**
 * Status-bar chip: a relay on this Mac is serving this Claude
 * subscription to other machines.
 *
 * The **ambient** tier for that state, and its only surface
 * (`rules/design.md`'s signal budget). Settings → Relay is the pane,
 * not a second indicator. Renders nothing while nothing serves.
 *
 * Its own chip, not `RemoteServingChip`'s: that one is the remote
 * panel — code execution behind a password. This one is inference on
 * a subscription, behind device tokens. A user who sees one lit must
 * be able to tell which door is open.
 *
 * A failed status read keeps the chip lit on the last known state (see
 * `useRelayStatus`); the tooltip then says the state is not current.
 */
export function RelayServingChip() {
  const { t } = useTranslation("components");
  const s = useRelayStatus();
  if (!s?.serving) return null;

  const streams = s.live.activeStreams;
  // Render-if-nonzero: never "0 streams".
  const label =
    streams > 0
      ? t("relayServing.streaming", { count: streams })
      : t("relayServing.serving");

  return (
    <span
      className={`statusbar-chip${s.live.blocking ? " warn" : ""}`}
      title={s.stale ? t("relayServing.hintStale") : t("relayServing.hint")}
      aria-label={label}
      role="status"
    >
      <Glyph g={NF.server} />
      {label}
    </span>
  );
}
