import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { MouseEvent } from "react";
import { api } from "../api";
import type { DoctorSeverity, DoctorSnapshot } from "../api/cc-doctor";
import { triggerSettingsTab } from "../lib/networkPanelDeepLink";

/**
 * 10×10 status dot for the WindowChrome — surfaces `claude doctor`'s
 * own self-diagnostic at a glance. Sits between the NotificationBell
 * and the theme toggle.
 *
 * Why a dot, not a labeled chip: this is a low-information surface
 * by design. "Is my CC OK?" → a glance. Detail belongs in Cut 2's
 * pane.
 *
 * Severity → dot fill:
 *   healthy  → var(--ok)
 *   warning  → var(--warn)
 *   error    → var(--danger)
 *   loading  → var(--fg-faint) (initial mount before the first scrape returns)
 *
 * Cadence: 60 s poll, matches the backend cache TTL exactly so the
 * second poll bypasses pty work. Window focus also triggers a
 * refresh — when the user comes back to Claudepot, show them what's
 * changed.
 *
 * Click behavior: navigate to Settings → Health. Uses the same
 * two-event pattern as the NetworkUnreachablePanel deep-link
 * (claudepot:navigate-section to swap the section, then
 * claudepot:settingsTab to set the subtab), which covers both the
 * cold-mount (sessionStorage hint) and hot-mount (live listener)
 * paths in App.tsx and SettingsSection.tsx.
 *
 * Failure handling: if a scrape returns `parseStatus.kind === "failed"`
 * the dot stays on its last-known-good state — a transient parser
 * blip shouldn't blank the indicator. The dev-alert path in core
 * surfaces the failure to the developer separately.
 */
interface HealthPillProps {
  onMouseDown?: (e: MouseEvent<HTMLButtonElement>) => void;
}

const POLL_INTERVAL_MS = 60_000;

export function HealthPill({ onMouseDown }: HealthPillProps) {
  const [snapshot, setSnapshot] = useState<DoctorSnapshot | null>(null);
  // Cache the last successful (non-failed) snapshot so a transient
  // parse failure doesn't blank the dot.
  const lastGood = useRef<DoctorSnapshot | null>(null);

  const refresh = useCallback(async (force = false) => {
    try {
      const s = await api.ccDoctorSnapshot(force);
      setSnapshot(s);
      // Only promote a fully-clean parse to "last-known-good".
      // Stashing degraded snapshots here would mean a later failed
      // parse falls back to already-incomplete data — the whole
      // point of the cache is to step back to TRUSTED state. A
      // degraded snapshot can still render (via `display` below) on
      // its own turn, but it should never inherit the lastGood
      // slot from a prior clean parse.
      if (s.parseStatus.kind === "ok") {
        lastGood.current = s;
      }
    } catch {
      // IPC failed entirely — leave the dot on its last value.
    }
  }, []);

  useEffect(() => {
    void refresh(false);
    const t = setInterval(() => void refresh(false), POLL_INTERVAL_MS);
    const onFocus = () => void refresh(false);
    window.addEventListener("focus", onFocus);
    return () => {
      clearInterval(t);
      window.removeEventListener("focus", onFocus);
    };
  }, [refresh]);

  // Pick the snapshot to render: prefer current, fall back to the
  // last good one if the current is a failed parse.
  const display = useMemo<DoctorSnapshot | null>(() => {
    if (!snapshot) return null;
    if (snapshot.parseStatus.kind === "failed" && lastGood.current) {
      return lastGood.current;
    }
    return snapshot;
  }, [snapshot]);

  const severity: DoctorSeverity | "loading" = display?.severity ?? "loading";

  const tooltip = useMemo(() => buildTooltip(display, snapshot), [display, snapshot]);
  const ariaLabel = useMemo(() => {
    if (!display) return "Checking Claude CLI health…";
    const lvl =
      display.severity === "healthy"
        ? "healthy"
        : display.severity === "warning"
          ? "has warnings"
          : display.severity === "error"
            ? "has errors"
            : "status unknown";
    return `Claude CLI health: ${lvl}`;
  }, [display]);

  const onClick = useCallback(() => {
    // Navigate to Settings → Health. Fire the section swap first
    // (sets the active primary nav to Settings), then the subtab
    // hint (selects the Health tab inside Settings). Both events
    // are idempotent — already on the section/tab is a no-op.
    window.dispatchEvent(
      new CustomEvent("claudepot:navigate-section", {
        detail: { id: "settings" },
      }),
    );
    triggerSettingsTab("health");
  }, []);

  return (
    <button
      type="button"
      onClick={onClick}
      onMouseDown={onMouseDown}
      title={tooltip}
      aria-label={ariaLabel}
      style={{
        // Bare circle — no chip chrome, no border. Sized to match
        // the bell's visual weight at this scale (the bell icon
        // renders at ~14px effective ink).
        width: "var(--sp-10)",
        height: "var(--sp-10)",
        padding: 0,
        background: dotColor(severity),
        border: "none",
        borderRadius: "var(--r-pill)",
        cursor: "pointer",
        flexShrink: 0,
      }}
    />
  );
}

function dotColor(severity: DoctorSeverity | "loading"): string {
  // Tokens guaranteed by src/styles/tokens.css — no literal
  // fallbacks (those'd diverge from the source of truth if the
  // tokens ever shift). `loading` and `unknown` both reuse
  // --fg-faint: same surface meaning ("no verdict yet"), different
  // provenance (loading = pre-first-scrape; unknown = scrape ran
  // and produced no signal).
  switch (severity) {
    case "healthy":
      return "var(--ok)";
    case "warning":
      return "var(--warn)";
    case "error":
      return "var(--danger)";
    case "unknown":
    case "loading":
    default:
      return "var(--fg-faint)";
  }
}

function buildTooltip(
  display: DoctorSnapshot | null,
  current: DoctorSnapshot | null,
): string {
  if (!current) return "Checking Claude CLI health…";

  // Special-case a stale "we're showing the last-known-good"
  // tooltip — the user should know the freshness story.
  const isStale =
    current.parseStatus.kind === "failed" && display && display !== current;

  const lines: string[] = [];
  if (display) {
    // Differentiate "we have a version" from "we don't" so the
    // tooltip is honest. Without the cc_version, the previous
    // "claude version unknown" copy invited the reader to think
    // CC itself was broken — but the truth is we couldn't measure.
    if (display.ccVersion) {
      const t = display.installType ? ` (${display.installType})` : "";
      lines.push(`claude ${display.ccVersion}${t}`);
    } else {
      lines.push("Couldn’t read claude doctor");
    }

    const flagged = display.sections.filter(
      (s) => s.severity !== "healthy" && s.severity !== "unknown",
    );
    // "No issues reported." is a positive claim — only emit it when
    // the scrape actually completed cleanly. A probe-backed snapshot
    // with an empty sections list and `parseStatus.kind !== "ok"`
    // means we never measured the issues, not that there are none.
    const parseOk = display.parseStatus.kind === "ok";
    if (display.ccVersion && parseOk && flagged.length === 0) {
      lines.push("No issues reported.");
    } else if (!parseOk && flagged.length === 0) {
      lines.push("(health check incomplete — refresh to retry)");
    } else {
      for (const s of flagged) {
        const dot = s.severity === "error" ? "✘" : "⚠";
        lines.push(`${dot} ${s.title}`);
      }
    }
  }

  if (isStale) {
    lines.push("(latest scrape failed to parse; showing last-known-good)");
  }

  return lines.join("\n");
}
