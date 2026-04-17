import { useCallback } from "react";
import { Monitor, Terminal, Play, LogIn } from "lucide-react";
import type { AccountSummary, UsageWindow } from "../types";

function formatResetTime(iso: string): string {
  const d = new Date(iso);
  return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

export function SidebarAccountItem({
  account: a, active, fiveHour, cliBusy, reBusy,
  onSelect, onSwitchCli, onLogin, onContextMenu, onBadgeContextMenu,
}: {
  account: AccountSummary;
  active: boolean;
  fiveHour: UsageWindow | null;
  cliBusy: boolean;
  reBusy: boolean;
  onSelect: () => void;
  onSwitchCli: () => void;
  onLogin: () => void;
  onContextMenu?: (e: React.MouseEvent) => void;
  /**
   * Right-click specifically on the status dot / usage bar. Distinct
   * from the row's full-item menu so we can show token/usage-scoped
   * actions (Verify now, Copy token status, Refresh usage).
   */
  onBadgeContextMenu?: (e: React.MouseEvent) => void;
}) {
  const tokenKind = a.drift ? "bad"
    : a.token_status.startsWith("valid") ? "ok"
    : a.token_status === "expired" ? "bad" : "warn";
  const dotTitle = a.drift
    ? `DRIFT — blob authenticates as ${a.verified_email}`
    : a.token_status;
  const fiveHourPct = fiveHour?.utilization ?? null;

  const stopClick = useCallback(
    (e: React.MouseEvent, fn: () => void) => { e.stopPropagation(); fn(); },
    [],
  );
  // Enter/Space on a focusable child (switch-CLI button, login button)
  // must not bubble to the parent item's own Enter/Space handler,
  // otherwise the user triggers select+action simultaneously.
  const stopKeyActivation = useCallback((e: React.KeyboardEvent) => {
    if (e.key === "Enter" || e.key === " ") e.stopPropagation();
  }, []);

  return (
    <div
      className={`sidebar-item ${active ? "active" : ""}`}
      role="option"
      aria-selected={active}
      onClick={onSelect}
      onContextMenu={onContextMenu}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") { e.preventDefault(); onSelect(); }
      }}
      tabIndex={0}
    >
      <span
        className={`status-dot ${tokenKind}`}
        title={dotTitle}
        onContextMenu={
          onBadgeContextMenu
            ? (e) => {
                e.stopPropagation();
                onBadgeContextMenu(e);
              }
            : undefined
        }
      />
      <div style={{ flex: 1, minWidth: 0 }}>
        <div className="sidebar-item-row">
          <span className="sidebar-item-text">{a.email}</span>
          <span className="sidebar-item-badges">
            {a.is_cli_active && (
              <Terminal size={13} strokeWidth={2.5} className="slot-icon cli active"
                aria-label="Active CLI account" />
            )}
            {a.is_desktop_active && (
              <Monitor size={13} strokeWidth={2.5} className="slot-icon desktop active"
                aria-label="Active Desktop account" />
            )}
            {!a.is_cli_active && a.credentials_healthy && (
              <button className="sidebar-switch-btn" disabled={cliBusy}
                onClick={(e) => stopClick(e, onSwitchCli)}
                onKeyDown={stopKeyActivation}
                title="Switch CLI to this account" aria-label={`Switch CLI to ${a.email}`}>
                <Play size={11} strokeWidth={2.5} />
              </button>
            )}
            {!a.credentials_healthy && !a.is_cli_active && (
              <button className="sidebar-switch-btn login" disabled={reBusy}
                onClick={(e) => stopClick(e, onLogin)}
                onKeyDown={stopKeyActivation}
                title={`Log in as ${a.email}`} aria-label={`Log in as ${a.email}`}>
                <LogIn size={11} strokeWidth={2} />
              </button>
            )}
          </span>
        </div>
        <div className="sidebar-item-meta">
          {a.org_name ?? "personal"}
          {a.subscription_type ? ` · ${a.subscription_type}` : ""}
        </div>
        {fiveHourPct !== null && (
          <div
            className="usage-bar-row"
            onContextMenu={
              onBadgeContextMenu
                ? (e) => {
                    e.stopPropagation();
                    onBadgeContextMenu(e);
                  }
                : undefined
            }
          >
            <div className="usage-bar-container">
              <div className={`usage-bar-fill ${fiveHourPct >= 80 ? "high" : ""}`}
                style={{ width: `${Math.min(fiveHourPct, 100)}%` }} />
            </div>
            <span className={`usage-bar-label ${fiveHourPct >= 80 ? "high" : ""}`}>
              {Math.round(fiveHourPct)}%
              {fiveHour?.resets_at && <> · resets {formatResetTime(fiveHour.resets_at)}</>}
            </span>
          </div>
        )}
      </div>
    </div>
  );
}
