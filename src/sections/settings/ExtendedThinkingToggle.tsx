import { useCallback, useEffect, useId, useState } from "react";
import { api } from "../../api";
import type { ThinkingState } from "../../api/thinking";
import { toastError } from "../../lib/toastError";

// Settings → General → Claude Code behavior.
//
// "Extended thinking by default" writes CC's user-level
// `alwaysThinkingEnabled`. On is CC's default (represented by the key's
// absence), so enabling clears the key and disabling writes `false` —
// matching CC's own /config UI. Turning it off trades some response
// quality for lower latency and token cost on new sessions.
//
// Read-only when MAX_THINKING_TOKENS is set — that env var hard-
// overrides the setting, so the toggle is disabled and the reason is
// stated inline (design.md: "disabled buttons state a reason inline").
// This is only the default for NEW sessions; per-process `--thinking` /
// `--max-thinking-tokens` args and project/policy layers live elsewhere.
//
// Markup mirrors CompanionArtifactToggle so it drops into the General
// pane's idiom without coupling to SettingsSection's local helpers.
export function ExtendedThinkingToggle({
  pushToast,
}: {
  pushToast: (kind: "info" | "error", text: string) => void;
}) {
  const [state, setState] = useState<ThinkingState | null>(null);
  const [busy, setBusy] = useState(false);
  const hintId = useId();

  const refresh = useCallback(async () => {
    try {
      setState(await api.thinkingState());
    } catch (e) {
      toastError(pushToast, "Thinking setting load failed", e);
    }
  }, [pushToast]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const on = state ? state.effective : false;
  const locked = !!state && !state.user_writable; // env var override
  const disabled = !state || busy || locked;

  const setOn = async (nextOn: boolean) => {
    setBusy(true);
    try {
      const updated = await api.thinkingSet(nextOn);
      setState(updated);
      pushToast(
        "info",
        nextOn
          ? "Extended thinking on by default for new sessions."
          : "Extended thinking off by default — new sessions skip it.",
      );
    } catch (e) {
      toastError(pushToast, "Toggle extended thinking", e);
    } finally {
      setBusy(false);
    }
  };

  const hint = !state
    ? "Loading…"
    : locked
      ? "Overridden by the MAX_THINKING_TOKENS environment variable. Unset it to control this here."
      : "On by default: Claude thinks before responding on supported models — better quality, more latency and tokens. Turn off to make new sessions skip thinking by default. Only affects new sessions; per-session flags still win.";

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
          Extended thinking by default
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
          aria-checked={on}
          aria-label="Extended thinking by default"
          aria-describedby={hintId}
          aria-disabled={disabled || undefined}
          disabled={disabled}
          onClick={disabled ? undefined : () => void setOn(!on)}
          className="pm-focus"
          style={{
            width: "var(--toggle-track-w)",
            height: "var(--toggle-track-h)",
            borderRadius: "var(--r-pill)",
            background: on ? "var(--accent)" : "var(--bg-active)",
            border: `var(--bw-hair) solid ${
              on ? "var(--accent)" : "var(--line-strong)"
            }`,
            position: "relative",
            cursor: disabled ? "not-allowed" : "pointer",
            opacity: disabled ? "var(--opacity-disabled)" : 1,
            transition: "background var(--dur-base) var(--ease-linear)",
          }}
        >
          <span
            aria-hidden
            style={{
              position: "absolute",
              top: "var(--toggle-thumb-off)",
              left: on ? "var(--toggle-thumb-on)" : "var(--toggle-thumb-off)",
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
