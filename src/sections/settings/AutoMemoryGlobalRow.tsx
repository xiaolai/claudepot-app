import type { ReactElement, ReactNode } from "react";
import { useCallback, useEffect, useState } from "react";
import { api } from "../../api";
import type { AutoMemoryStateDto } from "../../api/memory";

type ToggleComponent = (props: {
  on: boolean;
  onChange: (next: boolean) => void;
  disabled?: boolean;
}) => ReactElement;
type RowComponent = (props: {
  label: string;
  hint?: string;
  children: ReactNode;
}) => ReactElement;

interface AutoMemoryGlobalRowProps {
  Row: RowComponent;
  Toggle: ToggleComponent;
  pushToast: (kind: "info" | "error", msg: string) => void;
}

/**
 * Global auto-memory toggle in Settings → General. Writes
 * `~/.claude/settings.json :: autoMemoryEnabled`. Read-only when an
 * env var (CLAUDE_CODE_DISABLE_AUTO_MEMORY / CLAUDE_CODE_SIMPLE) is
 * overriding — the row labels the override so the user knows why the
 * toggle won't move.
 *
 * The CC priority chain reads more sources than just userSettings,
 * but for the global Settings pane we only show the user-settings
 * value — per-project overrides live in Projects → Memory.
 */
export function AutoMemoryGlobalRow({
  Row,
  Toggle,
  pushToast,
}: AutoMemoryGlobalRowProps) {
  const [state, setState] = useState<AutoMemoryStateDto | null>(null);
  const [busy, setBusy] = useState(false);

  // Use the global-only resolver so home-dir `.claude/settings.json`
  // doesn't get read twice as both userSettings and projectSettings
  // (audit 2026-05 #3). The toggle writes to userSettings via
  // `auto_memory_set("user", ...)` which is unaffected.
  const refresh = useCallback(async () => {
    try {
      const next = await api.autoMemoryStateGlobal();
      setState(next);
    } catch (e) {
      pushToast("error", `Auto-memory state load failed: ${e}`);
    }
  }, [pushToast]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  if (!state) {
    return (
      <Row
        label="Auto-memory"
        hint="Loading…"
      >
        <Toggle on={false} onChange={() => {}} disabled />
      </Row>
    );
  }

  const overridden =
    state.decided_by === "env_disable" || state.decided_by === "env_simple";
  const userValue = state.user_settings_value;
  // Effective for the global row: env > user > default.
  const globalEffective =
    overridden ? state.effective : userValue ?? true;

  const setValue = async (next: boolean) => {
    setBusy(true);
    try {
      // The set call needs *some* project anchor for its DTO carrier
      // even when targeting user-settings. After the write we re-pull
      // global-only state so display stays consistent with refresh().
      await api.autoMemorySet("~", "user", next);
      const updated = await api.autoMemoryStateGlobal();
      setState(updated);
      pushToast(
        "info",
        next ? "Auto-memory enabled globally." : "Auto-memory disabled globally.",
      );
    } catch (e) {
      pushToast("error", `Toggle failed: ${e}`);
    } finally {
      setBusy(false);
    }
  };

  const hint = overridden
    ? `Overridden by ${state.decided_label}. Clear the env var to control this from here.`
    : "Lets CC build a per-project memory directory and auto-write topic files at session end. Per-project overrides live in Projects → Memory.";

  return (
    <Row label="Auto-memory" hint={hint}>
      <Toggle
        on={globalEffective}
        onChange={(next) => void setValue(next)}
        disabled={busy || overridden}
      />
    </Row>
  );
}
