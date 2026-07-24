import { useCallback, useEffect, useId, useState } from "react";
import { api } from "../../api";
import type { AutoDreamMode, AutoDreamState } from "../../api/auto-dream";
import { Button } from "../../components/primitives/Button";
import { useAppState } from "../../providers/AppStateProvider";

// Global → Memory, directly below Auto-memory.
//
// Writes `~/.claude/settings.json :: autoDreamEnabled` — background
// memory consolidation (Claude reviews recent sessions and distills
// them into memory). Three-state, not a switch: when the key is absent
// CC falls back to a server-side rollout flag Claudepot can't observe,
// so a binary On/Off would misreport the default.
//
//   Default → follow Claude Code's own rollout.
//   On / Off → force it.
//
// Consolidation also requires auto-memory to be on. When it's off the
// control is disabled with the dependency stated inline (design.md:
// "disabled buttons state a reason inline").
export function AutoDreamGlobalCard() {
  const { pushToast } = useAppState();
  const [state, setState] = useState<AutoDreamState | null>(null);
  const [busy, setBusy] = useState(false);
  const hintId = useId();

  const refresh = useCallback(async () => {
    try {
      setState(await api.autoDreamState());
    } catch (e) {
      pushToast("error", `Consolidation state load failed: ${e}`);
    }
  }, [pushToast]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const depOff = !!state && !state.auto_memory_enabled;
  const disabled = !state || busy || depOff;

  const setMode = async (next: AutoDreamMode) => {
    setBusy(true);
    try {
      setState(await api.autoDreamSet(next));
      pushToast(
        "info",
        next === "default"
          ? "Consolidation follows Claude Code's default."
          : next === "on"
            ? "Background consolidation on."
            : "Background consolidation off.",
      );
    } catch (e) {
      pushToast("error", `Toggle failed: ${e}`);
    } finally {
      setBusy(false);
    }
  };

  const description = !state
    ? "Loading…"
    : depOff
      ? "Requires auto-memory, which is off — consolidation won't run until it's enabled above."
      : "Lets Claude Code review recent sessions in the background and distill them into memory. Default follows Claude Code's own rollout; override it here.";

  const OPTIONS: { value: AutoDreamMode; label: string }[] = [
    { value: "default", label: "Default" },
    { value: "on", label: "On" },
    { value: "off", label: "Off" },
  ];
  const current: AutoDreamMode = state?.mode ?? "default";

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
        <div style={{ fontSize: "var(--fs-sm)", fontWeight: 500, color: "var(--fg)" }}>
          Background memory consolidation
        </div>
        <div
          id={hintId}
          style={{
            fontSize: "var(--fs-2xs)",
            color: depOff ? "var(--warn)" : "var(--fg-faint)",
            lineHeight: "var(--lh-body)",
          }}
        >
          {description}
        </div>
      </div>
      <div
        role="group"
        aria-label="Background memory consolidation"
        aria-describedby={hintId}
        style={{ display: "inline-flex", gap: "var(--sp-4)" }}
      >
        {OPTIONS.map((o) => (
          <Button
            key={o.value}
            size="sm"
            variant={current === o.value ? "subtle" : "ghost"}
            active={current === o.value}
            aria-pressed={current === o.value}
            disabled={disabled}
            onClick={() => void setMode(o.value)}
          >
            {o.label}
          </Button>
        ))}
      </div>
    </div>
  );
}
