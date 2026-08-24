/**
 * Settings → Remote — the LAN appliance's config and controls.
 *
 * ## Three states, not two
 *
 * `enabled` is a stored preference and survives a `kill -9`, so
 * "enabled but nothing is serving" is a real and reachable state. A
 * pane that rendered `enabled` as "Running" would lie in exactly the way
 * `remote::approval`'s runtime gate exists to avoid — it heartbeats
 * every 5s precisely *because* the preference is not liveness. Off /
 * enabled-not-serving / serving are three rows in the status block, and
 * collapsing the middle one is a review finding.
 *
 * ## Two liveness fields
 *
 * `serving` is the machine's heartbeat; `runningHere` is this process.
 * They differ when someone has a `claudepot remote serve` in a
 * terminal, and the pane must not offer Stop for a process it cannot
 * stop.
 *
 * ## Why this pane is "core" and not "advanced"
 *
 * It is where you revoke a lost phone. An emergency control you have to
 * go hunting for is a broken emergency control — the same reasoning
 * `panes.ts` already records for Retention.
 */
import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { remoteApi, type RemoteStatus } from "../../api";
import { Button } from "../../components/primitives/Button";
import { ConfirmDialog } from "../../components/ConfirmDialog";
import { Glyph } from "../../components/primitives/Glyph";
import { NF } from "../../icons";
import { renderError } from "../../lib/i18n-error";

type Props = {
  pushToast?: (kind: "info" | "error", text: string) => void;
};

/** Poll while the pane is open. The surface can go down without us —
 *  a bind that fails after start, or a CLI server exiting — and a
 *  status block that only refreshes on a button press would go on
 *  claiming the phone can reach this Mac. */
const POLL_MS = 4000;

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
      // Seeded from the server, not held as a controlled default: a
      // user who has typed into these fields must not have their text
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

  if (loadError) {
    return (
      <div className="pane">
        <h2>{t("remote.title")}</h2>
        <p className="pane-error" role="alert">
          {loadError}
        </p>
      </div>
    );
  }

  if (!status) {
    return (
      <div className="pane">
        <h2>{t("remote.title")}</h2>
        <p className="muted">{t("remote.loading")}</p>
      </div>
    );
  }

  // A map rather than a template key: `t()` is typed against the
  // English catalog, so a hand-built `remote.state.${x}` compiles even
  // when the entry does not exist and leaks the raw key at runtime.
  const stateKey = !status.enabled
    ? "off"
    : status.serving
      ? "serving"
      : "idle";
  const STATE_LABEL = {
    off: "remote.state.off",
    idle: "remote.state.idle",
    serving: "remote.state.serving",
  } as const;

  return (
    <div className="pane">
      <h2>{t("remote.title")}</h2>
      <p className="pane-intro">{t("remote.intro")}</p>

      {/* The two fail-loud stores. Both are refusals rather than
          warnings in core; the pane's job is to say why the buttons
          below will not work. */}
      {status.configRecovered && (
        <p className="pane-warning" role="alert">
          <Glyph g={NF.warn} /> {t("remote.configRecovered")}
        </p>
      )}
      {status.devicesRecovered && (
        <p className="pane-warning" role="alert">
          <Glyph g={NF.warn} /> {t("remote.devicesRecovered")}
        </p>
      )}

      <section className="pane-block">
        <h3>{t("remote.statusHeading")}</h3>
        <p className={`remote-state remote-state-${stateKey}`}>
          <strong>{t(STATE_LABEL[stateKey])}</strong>
        </p>
        {/* The middle state's whole point: the preference says yes and
            nothing is listening. Without this line the user sees
            "enabled" and assumes the phone can connect. */}
        {stateKey === "idle" && <p className="muted">{t("remote.idleHint")}</p>}
        {status.serving && !status.runningHere && (
          <p className="muted">{t("remote.servingElsewhere")}</p>
        )}
        {status.url && status.runningHere && (
          <p className="mono selectable">{status.url}</p>
        )}
        {status.lastError && (
          <p className="pane-error" role="alert">
            {status.lastError}
          </p>
        )}
        {/* Approval-from-the-phone is off while these are present, and
            everything else still works — so it has to be said here
            rather than discovered by tapping Allow and waiting. */}
        {status.warnings.map((w) => (
          <p key={w} className="pane-warning" role="alert">
            <Glyph g={NF.warn} /> {w}
          </p>
        ))}

        <div className="pane-actions">
          {status.runningHere ? (
            <Button
              variant="ghost"
              disabled={busy !== null}
              onClick={() =>
                run("stop", () => remoteApi.remoteStop(), t("remote.stopped"))
              }
            >
              {t("remote.stop")}
            </Button>
          ) : (
            <Button
              variant="solid"
              glyph={NF.play}
              disabled={busy !== null || !status.enabled || !status.passwordSet}
              onClick={() =>
                run("start", () => remoteApi.remoteStart(), t("remote.started"))
              }
            >
              {t("remote.start")}
            </Button>
          )}
        </div>
        {/* `rules/design.md`: a disabled button states its reason
            inline, next to the button, never in a tooltip. */}
        {!status.runningHere && !status.enabled && (
          <p className="muted">{t("remote.cannotStartDisabled")}</p>
        )}
        {!status.runningHere && status.enabled && !status.passwordSet && (
          <p className="muted">{t("remote.cannotStartNoPassword")}</p>
        )}

        {/* The trade in-process hosting makes, disclosed rather than
            discovered. See `remote_server`'s module docs. */}
        <p className="muted">{t("remote.quitStops")}</p>
      </section>

      <section className="pane-block">
        <h3>{t("remote.addressHeading")}</h3>
        <label>
          <span>{t("remote.bind")}</span>
          <input
            value={bind}
            onChange={(e) => setBind(e.target.value)}
            className="mono"
            spellCheck={false}
          />
        </label>
        <label>
          <span>{t("remote.port")}</span>
          <input
            value={port}
            onChange={(e) => setPort(e.target.value)}
            className="mono"
            inputMode="numeric"
          />
        </label>

        {status.bindError && (
          <p className="pane-error" role="alert">
            {status.bindError}
          </p>
        )}
        {/* `0.0.0.0` is accepted deliberately and returned as
            `every_interface` precisely so a caller has to say so —
            "Accepted, never silently", in core's words. The CLI prints
            a paragraph; this is the same thing in this register. */}
        {status.exposure === "every_interface" && (
          <p className="pane-warning" role="alert">
            <Glyph g={NF.warn} /> {t("remote.everyInterface")}
          </p>
        )}
        {status.requiresTls && (
          <p className="muted">{t("remote.tlsRequired")}</p>
        )}

        <div className="pane-actions">
          {status.enabled ? (
            <Button
              variant="ghost"
              disabled={busy !== null}
              onClick={() =>
                run(
                  "disable",
                  () => remoteApi.remoteDisable(),
                  t("remote.disabled"),
                )
              }
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
        </div>
        {!status.enabled && !status.passwordSet && (
          <p className="muted">{t("remote.cannotEnableNoPassword")}</p>
        )}
      </section>

      <section className="pane-block">
        <h3>{t("remote.passwordHeading")}</h3>
        <p className="muted">
          {status.passwordSet ? t("remote.passwordSet") : t("remote.passwordUnset")}
        </p>
        <p className="muted">{t("remote.passwordWhy")}</p>
        <label>
          <span>{t("remote.newPassword")}</span>
          <input
            type="password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            autoComplete="new-password"
          />
        </label>
        <div className="pane-actions">
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
                  // `rules/architecture.md` requires it of every Add
                  // modal and this is the same shape.
                  setPassword("");
                },
                t("remote.passwordSaved"),
              )
            }
          >
            {t("remote.setPassword")}
          </Button>
        </div>
      </section>

      <section className="pane-block">
        <h3>
          {t("remote.devicesHeading")} ({status.activeDevices})
        </h3>
        {status.devices.length === 0 ? (
          <p className="muted">{t("remote.noDevices")}</p>
        ) : (
          <ul className="remote-devices">
            {status.devices.map((d) => (
              <li key={d.id}>
                <span className="remote-device-name">{d.name}</span>
                <span className="muted mono">
                  {d.revokedAt
                    ? t("remote.deviceRevoked")
                    : d.expiresAt
                      ? t("remote.deviceSession")
                      : t("remote.devicePaired")}
                </span>
                {!d.revokedAt && (
                  <Button
                    variant="ghost"
                    disabled={busy !== null || status.devicesRecovered}
                    onClick={() =>
                      run(
                        `revoke-${d.id}`,
                        () => remoteApi.remoteRevokeDevice(d.id),
                        t("remote.deviceRevokedOk"),
                      )
                    }
                  >
                    {t("remote.revoke")}
                  </Button>
                )}
              </li>
            ))}
          </ul>
        )}
        <div className="pane-actions">
          <Button
            variant="ghost"
            disabled={
              busy !== null ||
              status.devicesRecovered ||
              status.activeDevices === 0
            }
            onClick={() => setConfirmRevokeAll(true)}
          >
            {t("remote.revokeAll")}
          </Button>
        </div>
        {status.devicesRecovered && (
          <p className="muted">{t("remote.cannotRevokeRecovered")}</p>
        )}
      </section>

      {confirmRevokeAll && (
        <ConfirmDialog
          title={t("remote.revokeAllTitle")}
          body={t("remote.revokeAllBody")}
          confirmLabel={t("remote.revokeAll")}
          onCancel={() => setConfirmRevokeAll(false)}
          onConfirm={() => {
            setConfirmRevokeAll(false);
            void run(
              "revoke-all",
              () => remoteApi.remoteRevokeAll(),
              t("remote.revokedAll"),
            );
          }}
        />
      )}
    </div>
  );
}
