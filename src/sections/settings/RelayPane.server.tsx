// Settings → Relay: the Status and Address sections. See RelayPane.tsx
// for what the pane is and why changes need the relay stopped.
import { useTranslation } from "react-i18next";

import { relayApi, type RelayStatus } from "../../api";
import { Button } from "../../components/primitives/Button";
import { DevBadge } from "../../components/primitives/DevBadge";
import { Glyph } from "../../components/primitives/Glyph";
import { Input } from "../../components/primitives/Input";
import { SectionLabel } from "../../components/primitives/SectionLabel";
import { NF } from "../../icons";
import type { RelayPaneController } from "./useRelayPaneController";
import { publicationLabel } from "./RelayPane.publishing";
import { DANGER, FAINT, NOTE, SECTION, WARN } from "./RelayPane.styles";
import { SettingsField } from "./SettingsField";

// Maps, not template keys: `t()` is typed against the English catalog,
// so a hand-built `relay.state.${x}` would compile with no entry.
const STATE_LABEL = {
  off: "relay.state.off",
  here: "relay.state.here",
  elsewhere: "relay.state.elsewhere",
} as const;
const STATE_COLOR = {
  off: "var(--fg-muted)",
  here: "var(--ok)",
  elsewhere: "var(--warn)",
} as const;

export type RelayStateKey = keyof typeof STATE_LABEL;

export function relayStateKey(s: RelayStatus): RelayStateKey {
  return !s.serving ? "off" : s.runningHere ? "here" : "elsewhere";
}

/** The port field as `relay_set_bind` takes it: `undefined` for "keep
 *  the stored port", a number for a valid one, `null` for anything else.
 *  `Number()` alone turned `"abc"` into NaN, which serialises as `null`
 *  — a successful save that silently kept the old port — and passed
 *  fractions and out-of-range values on to a `u16`. */
export function parsePort(text: string): number | undefined | null {
  const v = text.trim();
  if (v === "") return undefined;
  if (!/^\d+$/.test(v)) return null;
  const n = Number(v);
  return n >= 1 && n <= 65535 ? n : null;
}

export function RelayStatusSection({ c, status }: { c: RelayPaneController; status: RelayStatus }) {
  const { t } = useTranslation("settings");
  const stateKey = relayStateKey(status);
  return (
    <section style={SECTION}>
      <SectionLabel>{t("relay.statusHeading")}</SectionLabel>
      <div style={{ fontSize: "var(--fs-md)", color: STATE_COLOR[stateKey], marginTop: "var(--sp-6)" }}>
        {t(STATE_LABEL[stateKey])}
      </div>
      {stateKey === "elsewhere" && <div style={NOTE}>{t("relay.servingElsewhere")}</div>}
      {status.addr && status.runningHere && (
        <div className="mono selectable" style={{ fontSize: "var(--fs-sm)", marginTop: "var(--sp-6)" }}>
          {status.addr}
        </div>
      )}
      {status.runningHere && status.live.activeStreams > 0 && (
        <div style={NOTE}>{t("relay.activeStreams", { count: status.live.activeStreams })}</div>
      )}
      {status.lastError && (
        <div style={DANGER} role="alert">
          {status.lastError}
        </div>
      )}
      {status.warnings.map((w) => {
        // Named as the Published list names it; the id is a slug, so it
        // sits behind the DevBadge.
        const p = status.publications.find((x) => x.id === w.upstream);
        const name = p ? publicationLabel(p, t) : t("relay.publicationGone");
        return (
          <div key={`${w.upstream}:${w.code}`} style={WARN} role="alert">
            <Glyph g={NF.warn} /> {t("relay.startWarning", { name, code: w.code })}{" "}
            <DevBadge>{w.upstream}</DevBadge>
          </div>
        );
      })}
      {!status.serving && status.publications.length === 0 && (
        <div style={NOTE}>{t("relay.nothingPublished")}</div>
      )}

      <div className="remote-actions">
        {status.runningHere ? (
          <Button
            variant="ghost"
            disabled={c.busy !== null}
            onClick={() => c.run("stop", () => relayApi.relayStop(), t("relay.stopped"))}
          >
            {c.busy === "stop" ? t("relay.stopping") : t("relay.stop")}
          </Button>
        ) : (
          <Button
            variant="solid"
            glyph={NF.play}
            disabled={c.busy !== null || status.serving}
            onClick={() => c.run("start", () => relayApi.relayStart(), t("relay.started"))}
          >
            {c.busy === "start" ? t("relay.starting") : t("relay.start")}
          </Button>
        )}
        {/* A start or stop in flight is its own label; any other change
            in flight disables this button too, and says so here. */}
        {stateKey === "elsewhere" ? (
          <span style={FAINT}>{t("relay.cannotStartElsewhere")}</span>
        ) : (
          c.busy !== null &&
          c.busy !== "start" &&
          c.busy !== "stop" && <span style={FAINT}>{t("relay.busyReason")}</span>
        )}
      </div>
      <div style={FAINT}>{t("relay.quitStops")}</div>
    </section>
  );
}

export function RelayAddressSection({ c, status }: { c: RelayPaneController; status: RelayStatus }) {
  const { t } = useTranslation("settings");
  // Every change but revoke is refused while anything serves.
  const locked = status.serving;
  const bindBlank = c.bind.trim() === "";
  const port = parsePort(c.port);
  const portInvalid = port === null;
  return (
    <section style={SECTION}>
      <SectionLabel>{t("relay.addressHeading")}</SectionLabel>
      <div className="remote-fields">
        <SettingsField label={t("relay.bind")}>
          <Input
            value={c.bind}
            onChange={(e) => c.setBind(e.target.value)}
            spellCheck={false}
            aria-label={t("relay.bind")}
          />
        </SettingsField>
        <SettingsField label={t("relay.port")}>
          <Input
            value={c.port}
            onChange={(e) => c.setPort(e.target.value)}
            inputMode="numeric"
            aria-label={t("relay.port")}
            aria-invalid={portInvalid || undefined}
            style={{ width: "var(--sp-96)" }}
          />
        </SettingsField>
      </div>
      {portInvalid && (
        <div style={DANGER} role="alert">
          {t("relay.portInvalid")}
        </div>
      )}
      {status.bindError && (
        <div style={DANGER} role="alert">
          {status.bindError}
        </div>
      )}
      {status.exposure === "every_interface" && (
        <div style={WARN} role="alert">
          <Glyph g={NF.warn} /> {t("relay.everyInterface")}
        </div>
      )}
      <div style={FAINT}>{t("relay.tlsAlways")}</div>
      <div className="remote-actions">
        <Button
          variant="ghost"
          disabled={c.busy !== null || locked || bindBlank || portInvalid}
          onClick={() => {
            if (port === null) return;
            void c.run("bind", () => relayApi.relaySetBind(c.bind.trim(), port), t("relay.bindSaved"));
          }}
        >
          {c.busy === "bind" ? t("relay.savingBind") : t("relay.saveBind")}
        </Button>
        {/* One reason, the one the user acts on first. An invalid port
            states its own above, at the field; a save in flight is the
            button's own label. */}
        {locked ? (
          <span style={FAINT}>{t("relay.lockedReason")}</span>
        ) : c.busy !== null && c.busy !== "bind" ? (
          <span style={FAINT}>{t("relay.busyReason")}</span>
        ) : (
          bindBlank && <span style={FAINT}>{t("relay.bindRequired")}</span>
        )}
      </div>
    </section>
  );
}
