import { useCallback, useEffect, useState } from "react";
import { api } from "../../api";
import type { FastModeState } from "../../api/fastMode";
import { compactModelLabel } from "../../lib/modelLabel";
import { toastError } from "../../lib/toastError";
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
  const [state, setState] = useState<FastModeState | null>(null);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(async () => {
    try {
      setState(await api.fastModeState());
    } catch (e) {
      toastError(pushToast, "Fast mode setting load failed", e);
    }
  }, [pushToast]);

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
        next
          ? "Fast mode on — new Opus sessions run faster and bill to usage credits."
          : "Fast mode off — new sessions run at standard Opus speed and rates.",
      );
    } catch (e) {
      toastError(pushToast, "Toggle fast mode", e);
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
          ? "Fast mode now resets each session — re-enable it with /fast."
          : "Fast mode preference now persists across sessions.",
      );
    } catch (e) {
      toastError(pushToast, "Toggle per-session opt-in", e);
    } finally {
      setBusy(false);
    }
  };

  const models = state
    ? state.facts.models.map(compactModelLabel).join(" and ")
    : "";
  const rate = state
    ? `$${state.facts.input_per_mtok}/$${state.facts.output_per_mtok} per MTok`
    : "";

  const hint = !state
    ? "Loading…"
    : locked
      ? "Overridden by the CLAUDE_CODE_DISABLE_FAST_MODE environment variable. Unset it to control this here."
      : `Runs ${models} up to 2.5× faster at ${rate} instead of the standard rate — same model, same quality. Charged to usage credits, not your plan's included usage, from the first token. Needs usage credits enabled on the account; Team and Enterprise also need an owner to turn it on.`;

  return (
    <>
      <SettingToggleRow
        label="Fast mode"
        hint={hint}
        hintTone={locked ? "warn" : "muted"}
        checked={on}
        disabled={disabled}
        onChange={(next) => void setOn(next)}
      />
      <SettingToggleRow
        label="Require per-session opt-in"
        hint={
          state?.per_session_opt_in
            ? "Every new session starts with fast mode off; re-enable it with /fast. Your saved preference is kept."
            : "Fast mode persists across sessions once turned on. Enable this to make each session start with it off instead."
        }
        checked={state?.per_session_opt_in ?? false}
        disabled={!state || busy}
        onChange={(next) => void setPerSession(next)}
        indent
      />
    </>
  );
}
