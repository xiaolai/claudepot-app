/**
 * Settings → Remote — the LAN appliance's config, controls, and the
 * quick prompts its composer shows.
 *
 * ## Three states, not two
 *
 * `enabled` is a stored preference and survives a `kill -9`, so
 * "enabled but nothing is serving" is a real and reachable state. A pane
 * that rendered `enabled` as "Serving" would lie in exactly the way
 * `remote::approval`'s runtime gate exists to avoid — it heartbeats
 * every 5s precisely *because* the preference is not liveness. Off /
 * enabled-not-serving / serving are three distinct renderings, and
 * collapsing the middle one is a review finding.
 *
 * ## Two liveness fields
 *
 * `serving` is the machine's heartbeat; `runningHere` is this process.
 * They differ when someone has a `claudepot remote serve` in a terminal,
 * and the pane must not offer Stop for a process it cannot stop.
 *
 * ## Quick prompts live here
 *
 * They are the chips above the *remote panel's* composer, and they mean
 * nothing anywhere else in this app — so a top-level Settings pane of
 * their own put one surface's detail beside Retention and Health. Folded
 * in as a section instead. `QuickPromptsPane` stays its own component:
 * its editor, its CSS and its behaviour were all fine, and what was
 * wrong was only where it sat in the nav.
 *
 * ## Why this pane is "core" and not "advanced"
 *
 * It is where you revoke a lost phone. An emergency control you have to
 * go hunting for is a broken emergency control — the same reasoning
 * `panes.ts` already records for Retention.
 */
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { remoteApi, type RemoteDevice, type RemoteStatus } from "../../api";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import { Button } from "../../components/primitives/Button";
import { Glyph } from "../../components/primitives/Glyph";
import { Input } from "../../components/primitives/Input";
import { SectionLabel } from "../../components/primitives/SectionLabel";
import { NF } from "../../icons";
import { renderError } from "../../lib/i18n-error";
import { QuickPromptsPane } from "./QuickPromptsPane";

type Props = {
  pushToast?: (kind: "info" | "error", text: string) => void;
};

/** Poll while the pane is open. The surface can go down without us — a
 *  bind that fails after start, or a CLI server exiting — and a status
 *  block that only refreshed on a button press would go on claiming the
 *  phone can reach this Mac. */
const POLL_MS = 4000;

/**
 * The pane's four text registers, from tokens.
 *
 * `.claude/rules/design.md` makes `tokens.css` the only place a value is
 * declared and calls a literal a review finding. Structural repetition
 * (rows, footers, the device list) goes to `styles/components/settings.css`
 * beside `qp-*`; one-off prose stays inline, the way `RetentionPane` does
 * it.
 */
const NOTE = {
  fontSize: "var(--fs-sm)",
  color: "var(--fg-muted)",
  lineHeight: "var(--lh-body)",
  marginTop: "var(--sp-6)",
} as const;
const FAINT = { ...NOTE, color: "var(--fg-faint)" } as const;
const WARN = { ...NOTE, color: "var(--warn)" } as const;
const DANGER = { ...NOTE, color: "var(--danger)" } as const;

const STATE_LABEL = {
  off: "remote.state.off",
  idle: "remote.state.idle",
  serving: "remote.state.serving",
} as const;

const STATE_COLOR = {
  off: "var(--fg-muted)",
  idle: "var(--warn)",
  serving: "var(--ok)",
} as const;

export function RemotePane({ pushToast }: Props) {
  const { t } = useTranslation("settings");
  const [status, setStatus] = useState<RemoteStatus | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [password, setPassword] = useState("");
  const [bind, setBind] = useState("");
  const [port, setPort] = useState("");
  const [confirmRevokeAll, setConfirmRevokeAll] = useState(false);

  const load = useCallback(async () => {
    try {
      const s = await remoteApi.remoteStatus();
      setStatus(s);
      setLoadError(null);
      // Seeded from the server, not held as a controlled default: a user
      // who has typed into these fields must not have their text
      // replaced by a poll four seconds later.
      setBind((b) => (b === "" ? s.bind : b));
      setPort((p) => (p === "" ? String(s.port) : p));
    } catch (e) {
      setLoadError(renderError(e, t("remote.loadFailed")));
    }
  }, [t]);

  useEffect(() => {
    void load();
    const id = window.setInterval(() => void load(), POLL_MS);
    return () => window.clearInterval(id);
  }, [load]);

  const run = async (key: string, fn: () => Promise<unknown>, okMsg?: string) => {
    setBusy(key);
    try {
      await fn();
      if (okMsg) pushToast?.("info", okMsg);
      await load();
    } catch (e) {
      pushToast?.("error", renderError(e, t("remote.actionFailed")));
    } finally {
      setBusy(null);
    }
  };

  // No heading here: `SettingsSection` already renders the pane title
  // from the registry, and a second one read as a repeated heading.
  if (loadError) {
    return (
      <div style={DANGER} role="alert">
        {loadError}
      </div>
    );
  }
  if (!status) return <div style={FAINT}>{t("remote.loading")}</div>;

  const stateKey = !status.enabled ? "off" : status.serving ? "serving" : "idle";

  return (
    <div>
      <div style={NOTE}>{t("remote.intro")}</div>

      {/* The two fail-loud stores. Both are refusals rather than
          warnings in core; the pane's job is to say why the buttons
          below will not work. */}
      {status.configRecovered && (
        <div style={WARN} role="alert">
          <Glyph g={NF.warn} /> {t("remote.configRecovered")}
        </div>
      )}
      {status.devicesRecovered && (
        <div style={WARN} role="alert">
          <Glyph g={NF.warn} /> {t("remote.devicesRecovered")}
        </div>
      )}

      <SectionLabel>{t("remote.statusHeading")}</SectionLabel>
      <div
        style={{
          fontSize: "var(--fs-md)",
          color: STATE_COLOR[stateKey],
          marginTop: "var(--sp-6)",
        }}
      >
        {/* A map, not a template key: `t()` is typed against the English
            catalog, so a hand-built `remote.state.${x}` would compile
            with no entry and leak the raw key at runtime. */}
        {t(STATE_LABEL[stateKey])}
      </div>
      {/* The middle state's whole point: the preference says yes and
          nothing is listening. Without this line the user sees "enabled"
          and assumes the phone can connect. */}
      {stateKey === "idle" && <div style={NOTE}>{t("remote.idleHint")}</div>}
      {status.serving && !status.runningHere && (
        <div style={NOTE}>{t("remote.servingElsewhere")}</div>
      )}
      {status.url && status.runningHere && (
        <div
          className="mono selectable"
          style={{ fontSize: "var(--fs-sm)", marginTop: "var(--sp-6)" }}
        >
          {status.url}
        </div>
      )}
      {status.lastError && (
        <div style={DANGER} role="alert">
          {status.lastError}
        </div>
      )}
      {/* Approval-from-the-phone is off while these are present, and
          everything else still works — so it has to be said here rather
          than discovered by tapping Allow and waiting. */}
      {status.warnings.map((w) => (
        <div key={w} style={WARN} role="alert">
          <Glyph g={NF.warn} /> {w}
        </div>
      ))}

      <div className="remote-actions">
        {status.runningHere ? (
          <Button
            variant="ghost"
            disabled={busy !== null}
            onClick={() => run("stop", () => remoteApi.remoteStop(), t("remote.stopped"))}
          >
            {t("remote.stop")}
          </Button>
        ) : (
          <Button
            variant="solid"
            glyph={NF.play}
            disabled={busy !== null || !status.enabled || !status.passwordSet}
            onClick={() => run("start", () => remoteApi.remoteStart(), t("remote.started"))}
          >
            {t("remote.start")}
          </Button>
        )}
        {/* `rules/design.md`: a disabled button states its reason inline,
            next to the button, never in a tooltip. */}
        {!status.runningHere && !status.enabled && (
          <span style={FAINT}>{t("remote.cannotStartDisabled")}</span>
        )}
        {!status.runningHere && status.enabled && !status.passwordSet && (
          <span style={FAINT}>{t("remote.cannotStartNoPassword")}</span>
        )}
      </div>

      {/* The trade in-process hosting makes, disclosed rather than
          discovered. See `remote_server`'s module docs. */}
      <div style={FAINT}>{t("remote.quitStops")}</div>

      <SectionLabel>{t("remote.addressHeading")}</SectionLabel>
      <div className="remote-fields">
        <Field label={t("remote.bind")}>
          <Input
            value={bind}
            onChange={(e) => setBind(e.target.value)}
            spellCheck={false}
            aria-label={t("remote.bind")}
          />
        </Field>
        <Field label={t("remote.port")}>
          <Input
            value={port}
            onChange={(e) => setPort(e.target.value)}
            inputMode="numeric"
            aria-label={t("remote.port")}
            style={{ width: "var(--sp-96)" }}
          />
        </Field>
      </div>

      {status.bindError && (
        <div style={DANGER} role="alert">
          {status.bindError}
        </div>
      )}
      {/* `0.0.0.0` is accepted deliberately and returned as
          `every_interface` precisely so a caller has to say so —
          "Accepted, never silently", in core's words. The CLI prints a
          paragraph; this is the same thing in this register. */}
      {status.exposure === "every_interface" && (
        <div style={WARN} role="alert">
          <Glyph g={NF.warn} /> {t("remote.everyInterface")}
        </div>
      )}
      {status.requiresTls && <div style={FAINT}>{t("remote.tlsRequired")}</div>}

      <div className="remote-actions">
        {status.enabled ? (
          <Button
            variant="ghost"
            disabled={busy !== null}
            onClick={() => run("disable", () => remoteApi.remoteDisable(), t("remote.disabled"))}
          >
            {t("remote.disable")}
          </Button>
        ) : (
          <Button
            variant="solid"
            disabled={busy !== null || !status.passwordSet}
            onClick={() =>
              run(
                "enable",
                () =>
                  remoteApi.remoteEnable(
                    bind.trim() || undefined,
                    port.trim() ? Number(port.trim()) : undefined,
                  ),
                t("remote.enabled"),
              )
            }
          >
            {t("remote.enable")}
          </Button>
        )}
        {!status.enabled && !status.passwordSet && (
          <span style={FAINT}>{t("remote.cannotEnableNoPassword")}</span>
        )}
      </div>

      <SectionLabel>{t("remote.passwordHeading")}</SectionLabel>
      <div style={NOTE}>
        {status.passwordSet ? t("remote.passwordSet") : t("remote.passwordUnset")}
      </div>
      <div style={FAINT}>{t("remote.passwordWhy")}</div>
      <div className="remote-fields">
        <Field label={t("remote.newPassword")}>
          <Input
            type="password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            autoComplete="new-password"
            aria-label={t("remote.newPassword")}
          />
        </Field>
      </div>
      <div className="remote-actions">
        <Button
          variant="solid"
          disabled={busy !== null || password.length === 0}
          onClick={() =>
            run(
              "password",
              async () => {
                await remoteApi.remoteSetPassword(password);
                // Cleared in the same tick the call resolves, so React
                // state does not outlive the single bridge call —
                // `rules/architecture.md` requires it of every Add modal
                // and this is the same shape.
                setPassword("");
              },
              t("remote.passwordSaved"),
            )
          }
        >
          {t("remote.setPassword")}
        </Button>
      </div>

      <SectionLabel
        right={
          // render-if-nonzero: never ship "0 devices".
          status.activeDevices > 0 ? (
            <span className="mono-cap" style={{ color: "var(--fg-faint)" }}>
              {t("remote.activeCount", { count: status.activeDevices })}
            </span>
          ) : undefined
        }
      >
        {t("remote.devicesHeading")}
      </SectionLabel>
      {status.devices.length === 0 ? (
        <div style={FAINT}>{t("remote.noDevices")}</div>
      ) : (
        <ul className="remote-devices">
          {status.devices.map((d) => (
            <DeviceRow
              key={d.id}
              d={d}
              disabled={busy !== null || status.devicesRecovered}
              onRevoke={() =>
                run(
                  `revoke-${d.id}`,
                  () => remoteApi.remoteRevokeDevice(d.id),
                  t("remote.deviceRevokedOk"),
                )
              }
            />
          ))}
        </ul>
      )}
      <div className="remote-actions">
        <Button
          variant="ghost"
          danger
          disabled={busy !== null || status.devicesRecovered || status.activeDevices === 0}
          onClick={() => setConfirmRevokeAll(true)}
        >
          {t("remote.revokeAll")}
        </Button>
        {status.devicesRecovered && (
          <span style={FAINT}>{t("remote.cannotRevokeRecovered")}</span>
        )}
      </div>

      {/* The chips above the remote panel's composer. Here rather than
          in a pane of their own — they mean nothing outside this
          surface. */}
      <SectionLabel>{t("remote.promptsHeading")}</SectionLabel>
      <QuickPromptsPane pushToast={pushToast} />

      {confirmRevokeAll && (
        <ConfirmDialog
          title={t("remote.revokeAllTitle")}
          body={t("remote.revokeAllBody")}
          confirmLabel={t("remote.revokeAll")}
          confirmDanger
          onCancel={() => setConfirmRevokeAll(false)}
          onConfirm={() => {
            setConfirmRevokeAll(false);
            void run("revoke-all", () => remoteApi.remoteRevokeAll(), t("remote.revokedAll"));
          }}
        />
      )}
    </div>
  );
}

/** A labelled field. `Input` draws its own chrome, so this is only the
 *  caption above it — and a real `<label>`, so the caption is a click
 *  target and the field has an accessible name twice over. */
function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="remote-field">
      <span className="mono-cap" style={{ color: "var(--fg-faint)" }}>
        {label}
      </span>
      {children}
    </label>
  );
}

/**
 * One paired device or password-issued session.
 *
 * The three states are words, not colours — `rules/design.md`'s floor
 * says colour never carries meaning alone, and "revoked" versus
 * "session" versus "paired" is exactly the distinction a user has to be
 * able to read rather than infer.
 */
function DeviceRow({
  d,
  disabled,
  onRevoke,
}: {
  d: RemoteDevice;
  disabled: boolean;
  onRevoke: () => void;
}) {
  const { t } = useTranslation("settings");
  const kind = d.revokedAt
    ? t("remote.deviceRevoked")
    : d.expiresAt
      ? t("remote.deviceSession")
      : t("remote.devicePaired");
  return (
    <li>
      <span style={{ fontSize: "var(--fs-sm)", opacity: d.revokedAt ? 0.55 : 1 }}>
        {d.name}
      </span>
      <span className="mono-cap" style={{ color: "var(--fg-faint)" }}>
        {kind}
      </span>
      {!d.revokedAt && (
        <Button variant="ghost" danger disabled={disabled} onClick={onRevoke}>
          {t("remote.revoke")}
        </Button>
      )}
    </li>
  );
}
