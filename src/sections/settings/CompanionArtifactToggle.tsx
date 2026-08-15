import { useCallback, useEffect, useId, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api";
import type { ArtifactState } from "../../api/artifact-tool";
import { toastError } from "../../lib/i18n-error";

// Settings → General → Claude Code behavior.
//
// "Keep companion output local" turns CC's Artifact (cloud-publish)
// tool off, so charts / mini-apps Claude builds land as a local file
// instead of a private page in the signed-in account's claude.ai
// gallery. Because that gallery is per-account, artifacts published
// under one account are unreachable after switching — keeping them
// local sidesteps the whole problem.
//
// Writes `~/.claude/settings.json :: enableArtifact` (the same field
// CC's own /config UI toggles). Global-only: CC resolves this key from
// user / policy / flag settings, never a project layer, so one write
// covers every account that shares the CC config slot.
//
// Read-only when CLAUDE_CODE_DISABLE_ARTIFACT is set — the env var
// hard-overrides settings, so the toggle is disabled and the reason is
// stated inline (design.md: "disabled buttons state a reason inline").
//
// Row + switch markup mirror the `Row` / `Toggle` primitives local to
// SettingsSection so this drops into the General pane's idiom without
// coupling to those non-exported helpers.
export function CompanionArtifactToggle({
  pushToast,
}: {
  pushToast: (kind: "info" | "error", text: string) => void;
}) {
  const { t } = useTranslation("settings");
  const [state, setState] = useState<ArtifactState | null>(null);
  const [busy, setBusy] = useState(false);
  const hintId = useId();

  const refresh = useCallback(async () => {
    try {
      setState(await api.artifactToolState());
    } catch (e) {
      toastError(pushToast, t("companion.loadFailed"), e);
    }
  }, [pushToast, t]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // The switch reads as "keep output local" = the Artifact tool OFF.
  const local = state ? !state.enabled : false;
  const locked = !!state && !state.user_writable; // env var override
  const disabled = !state || busy || locked;

  const setLocal = async (nextLocal: boolean) => {
    setBusy(true);
    try {
      // nextLocal true  → keep local → Artifact tool disabled
      // nextLocal false → cloud (CC default) → Artifact tool enabled
      const updated = await api.artifactToolSet(!nextLocal);
      setState(updated);
      pushToast(
        "info",
        nextLocal ? t("companion.keptLocal") : t("companion.reenabled"),
      );
    } catch (e) {
      toastError(pushToast, t("companion.toggleFailed"), e);
    } finally {
      setBusy(false);
    }
  };

  const hint = !state
    ? t("shared.loading")
    : locked
      ? t("companion.lockedHint")
      : t("companion.hint");

  return (
    <div
      style={{
        display: "grid",
        gridTemplateColumns: "var(--settings-label-col) 1fr",
        gap: "var(--sp-16)",
        alignItems: "start",
        padding: "var(--sp-8) 0",
        borderBottom: "var(--bw-hair) solid var(--line)",
      }}
    >
      <div>
        <div style={{ fontSize: "var(--fs-sm)", color: "var(--fg)" }}>
          {t("companion.label")}
        </div>
        <div
          id={hintId}
          style={{
            fontSize: "var(--fs-xs)",
            color: locked ? "var(--warn)" : "var(--fg-faint)",
            marginTop: "var(--sp-3)",
            lineHeight: "var(--lh-body)",
          }}
        >
          {hint}
        </div>
      </div>
      <div style={{ display: "flex", alignItems: "center" }}>
        <button
          type="button"
          role="switch"
          aria-checked={local}
          aria-label={t("companion.label")}
          aria-describedby={hintId}
          aria-disabled={disabled || undefined}
          disabled={disabled}
          onClick={disabled ? undefined : () => void setLocal(!local)}
          className="pm-focus"
          style={{
            width: "var(--toggle-track-w)",
            height: "var(--toggle-track-h)",
            borderRadius: "var(--r-pill)",
            background: local ? "var(--accent)" : "var(--bg-active)",
            border: `var(--bw-hair) solid ${
              local ? "var(--accent)" : "var(--line-strong)"
            }`,
            position: "relative",
            opacity: disabled ? "var(--opacity-disabled)" : 1,
            transition: "background var(--dur-base) var(--ease-linear)",
          }}
        >
          <span
            aria-hidden
            style={{
              position: "absolute",
              top: "var(--toggle-thumb-off)",
              left: local ? "var(--toggle-thumb-on)" : "var(--toggle-thumb-off)",
              width: "var(--toggle-thumb-d)",
              height: "var(--toggle-thumb-d)",
              borderRadius: "50%",
              background: "var(--bg-raised)",
              boxShadow: "var(--shadow-thumb)",
              transition: "left var(--dur-base) var(--ease-linear)",
            }}
          />
        </button>
      </div>
    </div>
  );
}
