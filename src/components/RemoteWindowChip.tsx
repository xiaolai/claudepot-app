import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { listen } from "@tauri-apps/api/event";
import { Glyph } from "./primitives/Glyph";
import { NF } from "../icons";
import { api } from "../api";
import type { PeerInboundState } from "../api/peerInbound";

/**
 * Status-bar chip for the remote-control window — Claude Code's
 * `crossSessionInbound`.
 *
 * This is the **ambient** surface for that state, and the only one.
 * While the window is open, any local process that can reach a
 * session's socket can drive it without asking; the design rule that
 * ongoing state gets exactly one always-visible indicator is what makes
 * that safe to offer at all. Renders nothing when the gate is closed,
 * like every other render-if-nonzero segment in the bar.
 *
 * Two open states, deliberately distinct:
 *
 * - **managed** — a Claudepot grant holds a deadline. Shows the time
 *   left, counting down.
 * - **unmanaged** — `accept` is set but no grant is minding it, so
 *   nothing will ever close it. Either the user set it by hand or a
 *   grant record was lost. Shown in the warning tone with no timer,
 *   because "open forever" and "open for 12 more minutes" are not the
 *   same risk and must not look the same.
 */
export function RemoteWindowChip() {
  const { t } = useTranslation("components");
  const [state, setState] = useState<PeerInboundState | null>(null);

  const refresh = useCallback(async () => {
    try {
      setState(await api.peerInboundState());
    } catch {
      // A chip that cannot read its own state renders nothing rather
      // than guessing "closed" — guessing closed is the one wrong
      // answer here.
      setState(null);
    }
  }, []);

  useEffect(() => {
    void refresh();
    // Poll for the countdown. 30s is enough for a minutes-granularity
    // label and cheap (one file read); the orchestrator's event covers
    // the moment that actually matters.
    const timer = window.setInterval(() => void refresh(), 30_000);
    const unlisten = listen("peer-inbound-closed", () => void refresh());
    return () => {
      window.clearInterval(timer);
      void unlisten.then((f) => f());
    };
  }, [refresh]);

  if (!state?.open) return null;

  const minutes =
    state.remainingSecs != null ? Math.max(0, Math.ceil(state.remainingSecs / 60)) : null;

  const unmanaged = state.unmanagedOpen;
  const label = unmanaged
    ? t("remoteWindow.unmanaged")
    : t("remoteWindow.open", { minutes: minutes ?? 0 });

  return (
    <span
      className={`statusbar-chip${unmanaged ? " warn" : ""}`}
      // Colour never carries meaning alone: the label already says
      // which state this is, and the title spells out the consequence.
      title={
        unmanaged
          ? t("remoteWindow.unmanagedHint")
          : t("remoteWindow.openHint", { minutes: minutes ?? 0 })
      }
      role="status"
    >
      <Glyph g={NF.unlock} />
      {label}
    </span>
  );
}
