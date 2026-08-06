import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api";
import type { FastModeState } from "../../api/fastMode";
import { compactModelLabel } from "../../lib/modelLabel";
import { toastError } from "../../lib/i18n-error";
import { SettingToggleRow } from "./SettingToggleRow";

// Settings → General → Claude Code behavior.
//
// "Fast mode" writes CC's user-level `fastMode`. Off is CC's default
// (represented by the key's absence), so enabling writes `true` and
// disabling clears the key — the mirror of the extended-thinking
// toggle, whose default is on.
//
// This is the one behavior toggle that spends money directly: fast mode
// bills at a higher rate AND draws from usage credits rather than the
// plan's included usage. The hint says so in both places rather than
// hiding it in a tooltip — a switch that changes what you're charged
// has to disclose that on the surface.
//
// Read-only when CLAUDE_CODE_DISABLE_FAST_MODE is set — that env var
// hard-overrides the setting, so the switch is disabled and the reason
// is stated inline (design.md: "disabled buttons state a reason
// inline").
//
// Claudepot can't verify the other two requirements (usage credits
// enabled; owner enablement on Team/Enterprise) — neither is visible
// from disk — so the hint states them rather than implying a check.
export function FastModeToggle({
  pushToast,
}: {
  pushToast: (kind: "info" | "error", text: string) => void;
}) {
  const { t } = useTranslation("settings");
  const [state, setState] = useState<FastModeState | null>(null);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(async () => {
    try {
      setState(await api.fastModeState());
    } catch (e) {
      toastError(pushToast, t("fastMode.loadFailed"), e);
    }
  }, [pushToast, t]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const on = state?.effective ?? false;
  const locked = !!state && !state.user_writable; // env var override
  const disabled = !state || busy || locked;

  const setOn = async (next: boolean) => {
    // `busy` disables the switch, but React renders that a tick after
    // the click; two fast clicks can both reach here and resolve out of
    // order, leaving the switch showing the older write. Refuse
    // re-entry rather than relying on the rendered disabled state.
    if (busy) return;
    setBusy(true);
    try {
      setState(await api.fastModeSet(next));
      pushToast(
        "info",
        next ? t("fastMode.onToast") : t("fastMode.offToast"),
      );
    } catch (e) {
      toastError(pushToast, t("fastMode.toggleFailed"), e);
    } finally {
      setBusy(false);
    }
  };

  const setPerSession = async (next: boolean) => {
    if (busy) return;
    setBusy(true);
    try {
      setState(await api.fastModeSetPerSession(next));
      pushToast(
        "info",
        next
          ? t("fastMode.perSessionOnToast")
          : t("fastMode.perSessionOffToast"),
      );
    } catch (e) {
      toastError(pushToast, t("fastMode.perSessionFailed"), e);
    } finally {
      setBusy(false);
    }
  };

  const models = state
    ? state.facts.models.map(compactModelLabel).join(t("fastMode.joinAnd"))
    : "";
  const rate = state
    ? t("fastMode.rate", {
        input: state.facts.input_per_mtok,
        output: state.facts.output_per_mtok,
      })
    : "";

  const hint = !state
    ? t("shared.loading")
    : locked
      ? t("fastMode.lockedHint")
      : t("fastMode.hint", { models, rate });

  return (
    <>
      <SettingToggleRow
        label={t("fastMode.label")}
        hint={hint}
        hintTone={locked ? "warn" : "muted"}
        checked={on}
        disabled={disabled}
        onChange={(next) => void setOn(next)}
      />
      <SettingToggleRow
        label={t("fastMode.perSessionLabel")}
        hint={
          state?.per_session_opt_in
            ? t("fastMode.perSessionOnHint")
            : t("fastMode.perSessionOffHint")
        }
        checked={state?.per_session_opt_in ?? false}
        disabled={!state || busy}
        onChange={(next) => void setPerSession(next)}
        indent
      />
    </>
  );
}
