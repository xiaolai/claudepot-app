import { useCallback, useEffect, useId, useState } from "react";
import { api } from "../../api";
import type { AttributionModeKind, AttributionState } from "../../api/attribution";
import { Button } from "../../components/primitives/Button";
import { toastError } from "../../lib/toastError";

// Settings → General → Claude Code behavior.
//
// Controls whether Claude's attribution lands on git commits and pull
// requests, and what it says. Writes CC's user-level `attribution`
// object plus the deprecated-but-still-honored `includeCoAuthoredBy`
// guard in one atomic write (see
// claudepot_core::attribution_settings — the guard is required because
// CC's enhanced-PR path treats an empty `attribution.pr` as "not set").
//
//   Default → CC's "Co-Authored-By" trailer + "Generated with Claude Code".
//   Off     → nothing on commits or PRs.
//   Custom  → your own commit-trailer and PR-body text.
//
// Default/Off apply on click; Custom opens an editor and applies on Save.
export function AttributionControl({
  pushToast,
}: {
  pushToast: (kind: "info" | "error", text: string) => void;
}) {
  const [state, setState] = useState<AttributionState | null>(null);
  const [mode, setMode] = useState<AttributionModeKind>("default");
  const [commit, setCommit] = useState("");
  const [pr, setPr] = useState("");
  const [busy, setBusy] = useState(false);
  const hintId = useId();

  const hydrate = useCallback((s: AttributionState) => {
    setState(s);
    setMode(s.mode);
    setCommit(s.commit ?? "");
    setPr(s.pr ?? "");
  }, []);

  const refresh = useCallback(async () => {
    try {
      hydrate(await api.attributionState());
    } catch (e) {
      toastError(pushToast, "Attribution setting load failed", e);
    }
  }, [pushToast, hydrate]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const apply = async (
    next: AttributionModeKind,
    commitText?: string,
    prText?: string,
  ) => {
    setBusy(true);
    try {
      const updated = await api.attributionSet(next, commitText, prText);
      hydrate(updated);
      pushToast(
        "info",
        next === "default"
          ? "Attribution reset to Claude Code's default."
          : next === "off"
            ? "Attribution off — commits and PRs carry none."
            : "Custom attribution saved.",
      );
    } catch (e) {
      toastError(pushToast, "Set attribution", e);
    } finally {
      setBusy(false);
    }
  };

  const pickMode = (next: AttributionModeKind) => {
    setMode(next);
    // Default / Off have no text to author — apply immediately.
    // Custom opens the editor; the Save button commits it.
    if (next !== "custom") void apply(next);
  };

  const OPTIONS: { value: AttributionModeKind; label: string }[] = [
    { value: "default", label: "Default" },
    { value: "off", label: "Off" },
    { value: "custom", label: "Custom" },
  ];

  const dirty =
    mode === "custom" &&
    (commit !== (state?.commit ?? "") ||
      pr !== (state?.pr ?? "") ||
      state?.mode !== "custom");

  // A custom attribution with both fields empty writes the exact same
  // state as Off — so steer the user to the Off button instead of
  // letting Save silently produce it (which would read back as Off).
  const bothEmpty = commit === "" && pr === "";

  const areaStyle = {
    width: "100%",
    resize: "vertical" as const,
    minHeight: "var(--btn-h-lg)",
    padding: "var(--sp-6) var(--sp-8)",
    fontSize: "var(--fs-xs)",
    fontFamily: "inherit",
    color: "var(--fg)",
    background: "var(--bg-sunken)",
    border: "var(--bw-hair) solid var(--line)",
    borderRadius: "var(--r-1)",
    lineHeight: "var(--lh-body)",
  };

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
          Commit &amp; PR attribution
        </div>
        <div
          id={hintId}
          style={{
            fontSize: "var(--fs-xs)",
            color: "var(--fg-faint)",
            marginTop: "var(--sp-3)",
            lineHeight: "var(--lh-body)",
          }}
        >
          Whether Claude's attribution appears on git commits and pull
          requests, and what it says. Off removes the “Co-Authored-By”
          trailer and the “Generated with Claude Code” line everywhere.
        </div>
      </div>
      <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-8)" }}>
        <div
          role="group"
          aria-label="Attribution mode"
          aria-describedby={hintId}
          style={{ display: "inline-flex", gap: "var(--sp-4)" }}
        >
          {OPTIONS.map((o) => (
            <Button
              key={o.value}
              size="sm"
              variant={mode === o.value ? "subtle" : "ghost"}
              active={mode === o.value}
              aria-pressed={mode === o.value}
              disabled={busy || !state}
              onClick={() => pickMode(o.value)}
            >
              {o.label}
            </Button>
          ))}
        </div>

        {mode === "custom" && (
          <div style={{ display: "flex", flexDirection: "column", gap: "var(--sp-6)" }}>
            <label
              style={{
                display: "flex",
                flexDirection: "column",
                gap: "var(--sp-2)",
                fontSize: "var(--fs-2xs)",
                color: "var(--fg-faint)",
              }}
            >
              Commit trailer
              <textarea
                value={commit}
                onChange={(e) => setCommit(e.target.value)}
                placeholder="Co-Authored-By: You <you@example.com>"
                rows={2}
                disabled={busy}
                style={areaStyle}
              />
            </label>
            <label
              style={{
                display: "flex",
                flexDirection: "column",
                gap: "var(--sp-2)",
                fontSize: "var(--fs-2xs)",
                color: "var(--fg-faint)",
              }}
            >
              Pull-request body
              <textarea
                value={pr}
                onChange={(e) => setPr(e.target.value)}
                placeholder="Generated with AI"
                rows={2}
                disabled={busy}
                style={areaStyle}
              />
            </label>
            <div style={{ display: "flex", alignItems: "center", gap: "var(--sp-8)" }}>
              <Button
                size="sm"
                variant="outline"
                disabled={busy || !dirty || bothEmpty}
                onClick={() => void apply("custom", commit, pr)}
              >
                {busy ? "Saving…" : "Save custom attribution"}
              </Button>
              {bothEmpty && (
                <span style={{ fontSize: "var(--fs-2xs)", color: "var(--fg-faint)" }}>
                  Both fields empty — use Off instead.
                </span>
              )}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
