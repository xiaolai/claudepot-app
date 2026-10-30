import { useMemo } from "react";
import { useTranslation } from "react-i18next";

import { triggerSettingsTab } from "../lib/networkPanelDeepLink";
import type { StatusIssue } from "./useStatusIssues";
import { useRelayStatus } from "./useRelayStatus";

/**
 * The relay's **blocking** tier: the conditions that stop it serving
 * and that only the user can clear. Everything else a relay reports is
 * the log's (`useRelayErrorNotifications`) or the chip's.
 *
 * - `owner_conflict` — another process holds the account's refresh
 *   lease, so serving it would race Claude Code for the token.
 * - `identity_changed` — the account now signs in as someone else;
 *   publishing it again is the only way to accept that.
 * - `relay_ca_key_unavailable` — the relay's CA key cannot be read, so
 *   no start can succeed until it is repaired or reissued.
 *
 * Not dismissable: each blocks every request until resolved.
 */
export function useRelayIssues(setSection: (id: string) => void): StatusIssue[] {
  // Banner copy is resolved inside the memo, so the active language is
  // one of its inputs; without it a language switch would leave these
  // banners in the previous language until the relay's state changed.
  const { t, i18n } = useTranslation("common");
  const language = i18n.language;
  const s = useRelayStatus();
  const blocking = s?.live.blocking ?? null;
  const caUnavailable = s?.lastErrorCode === "relay_ca_key_unavailable";

  return useMemo(() => {
    const action = {
      label: t("status.relay.action"),
      onClick: () => {
        triggerSettingsTab("relay");
        setSection("settings");
      },
    };
    const out: StatusIssue[] = [];
    if (blocking) {
      out.push({
        id: `relay:${blocking}`,
        severity: "error",
        label: t(`status.relay.${blocking}.label`),
        detail: t(`status.relay.${blocking}.detail`),
        action,
      });
    }
    if (caUnavailable) {
      out.push({
        id: "relay:ca_key_unavailable",
        severity: "error",
        label: t("status.relay.caKeyUnavailable.label"),
        detail: t("status.relay.caKeyUnavailable.detail"),
        action,
      });
    }
    return out;
  }, [blocking, caUnavailable, setSection, t, language]);
}
