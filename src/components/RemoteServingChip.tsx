import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Glyph } from "./primitives/Glyph";
import { NF } from "../icons";
import { remoteApi } from "../api";

/**
 * Status-bar chip for the LAN appliance — is anything on this Mac
 * serving the remote panel right now.
 *
 * The **ambient** surface for that state, and the only one:
 * `rules/design.md`'s signal budget gives an ongoing state exactly one
 * always-visible indicator, and Settings → Remote is the pane, not a
 * second indicator. Renders nothing while nothing is serving, like
 * every other render-if-nonzero segment in the bar.
 *
 * ## Not the same chip as `RemoteWindowChip`
 *
 * That one is Claude Code's `crossSessionInbound` — a local gate on
 * peer messages. This one is an HTTP server on the network. They are
 * different capabilities with different blast radii, and folding them
 * into one chip would mean a user who saw it lit could not tell which
 * door was open.
 *
 * ## It reads liveness, not the preference
 *
 * `serving` is the heartbeat `remote::approval` already keeps, so a
 * server started from a terminal lights this too — which is correct: the
 * question the bar answers is "can something out there reach this Mac",
 * and the answer does not depend on who started it. `enabled` is
 * deliberately not consulted; it survives a `kill -9` and would leave
 * this lit over a dead server.
 *
 * ## A failed read is not "closed"
 *
 * Both render nothing today, which is a shape `RemoteWindowChip` was
 * fixed for — but the asymmetry there is real and applies here in
 * reverse. That chip guards a gate whose *open* state is the dangerous
 * one, so "I could not tell" must not look shut. Here the dangerous
 * state is also open, and an IPC read failing means the backend is
 * gone, which means the server this process hosts is gone too. Silence
 * is then accurate rather than a guess, and a warning chip for an app
 * that is itself failing would be noise on top of a bigger problem.
 */
const POLL_MS = 10_000;

export function RemoteServingChip() {
  const { t } = useTranslation("components");
  const [serving, setServing] = useState(false);
  const [everyInterface, setEveryInterface] = useState(false);
  // The interval and the mount read can be in flight together; without
  // a sequence number an older reply can land last and overwrite a
  // newer one — the bug `RemoteWindowChip` carries the same guard for.
  const seq = useRef(0);
  const alive = useRef(true);

  const refresh = useCallback(async () => {
    const mine = ++seq.current;
    try {
      const s = await remoteApi.remoteStatus();
      if (!alive.current || mine !== seq.current) return;
      setServing(s.serving);
      setEveryInterface(s.exposure === "every_interface");
    } catch {
      if (!alive.current || mine !== seq.current) return;
      setServing(false);
      setEveryInterface(false);
    }
  }, []);

  useEffect(() => {
    alive.current = true;
    void refresh();
    const id = window.setInterval(() => void refresh(), POLL_MS);
    return () => {
      alive.current = false;
      window.clearInterval(id);
    };
  }, [refresh]);

  if (!serving) return null;

  // Colour never carries meaning alone — `rules/design.md`'s
  // accessibility floor. The label says which of the two it is.
  const label = everyInterface
    ? t("remoteServing.everyInterface")
    : t("remoteServing.serving");

  return (
    <span
      className={`status-chip${everyInterface ? " status-chip-warn" : ""}`}
      title={label}
      aria-label={label}
    >
      <Glyph g={NF.globe} />
      <span>{label}</span>
    </span>
  );
}
