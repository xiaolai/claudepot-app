import { useCallback, useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { api } from "../../api";
import type { AutoMemoryStateDto } from "../../api/memory";
import { renderError } from "../../lib/i18n-error";
import { useAppState } from "../../providers/AppStateProvider";

// Global → Memory.
//
// Writes `~/.claude/settings.json :: autoMemoryEnabled`. Read-only
// when an env var (CLAUDE_CODE_DISABLE_AUTO_MEMORY /
// CLAUDE_CODE_SIMPLE) is overriding — the override is labeled in
// the body so the user knows why the toggle won't move.
//
// The CC priority chain reads more sources than just userSettings,
// but for the global pane we only show the user-settings value —
// per-project overrides live in Projects → Memory.
export function AutoMemoryGlobalCard() {
  const { t } = useTranslation("global");
  const { pushToast } = useAppState();
  const [state, setState] = useState<AutoMemoryStateDto | null>(null);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(async () => {
    try {
      const next = await api.autoMemoryStateGlobal();
      setState(next);
    } catch (e) {
      pushToast("error", renderError(e, t("memory.auto.loadFailed")));
    }
  }, [pushToast, t]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const overridden =
    !!state &&
    (state.decided_by === "env_disable" || state.decided_by === "env_simple");
  const userValue = state?.user_settings_value;
  const globalEffective = state
    ? overridden
      ? state.effective
      : userValue ?? true
    : false;

  const setValue = async (next: boolean) => {
    setBusy(true);
    try {
      // The set call needs *some* project anchor for its DTO carrier
      // even when targeting user-settings. After the write we re-pull
      // global-only state so display stays consistent.
      await api.autoMemorySet("~", "user", next);
      const updated = await api.autoMemoryStateGlobal();
      setState(updated);
      pushToast(
        "info",
        next
          ? t("memory.auto.enabledToast")
          : t("memory.auto.disabledToast"),
      );
    } catch (e) {
      pushToast("error", renderError(e, t("memory.auto.toggleFailed")));
    } finally {
      setBusy(false);
    }
  };

  const description = !state
    ? t("memory.auto.loading")
    : overridden
    ? t("memory.auto.overridden", { label: state.decided_label })
    : t("memory.auto.description");

  const disabled = !state || busy || overridden;

  return (
    <div
      style={{
        background: "var(--bg-raised)",
        border: "var(--bw-hair) solid var(--line)",
        borderRadius: "var(--r-2)",
        padding: "var(--sp-10) var(--sp-12)",
        display: "grid",
        gridTemplateColumns: "1fr auto",
        alignItems: "center",
        gap: "var(--sp-12)",
      }}
    >
      <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-2)" }}>
        <div
          style={{
            fontSize: "var(--fs-sm)",
            fontWeight: 500,
            color: "var(--fg)",
          }}
        >
          {t("memory.auto.title")}
        </div>
        <div
          style={{
            fontSize: "var(--fs-2xs)",
            color: "var(--fg-faint)",
            lineHeight: "var(--lh-body)",
          }}
        >
          {description}
        </div>
      </div>
      <button
        type="button"
        role="switch"
        aria-checked={globalEffective}
        aria-label={t("memory.auto.title")}
        aria-disabled={disabled || undefined}
        disabled={disabled}
        onClick={
          disabled ? undefined : () => void setValue(!globalEffective)
        }
        style={{
          width: "var(--toggle-track-w)",
          height: "var(--toggle-track-h)",
          borderRadius: "var(--r-pill)",
          background: globalEffective ? "var(--accent)" : "var(--bg-active)",
          border: `var(--bw-hair) solid ${
            globalEffective ? "var(--accent)" : "var(--line-strong)"
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
            left: globalEffective
              ? "var(--toggle-thumb-on)"
              : "var(--toggle-thumb-off)",
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
  );
}
