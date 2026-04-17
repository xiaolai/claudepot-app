import { useCallback } from "react";
import { Monitor, Terminal, Play, LogIn, AlertCircle, Clock } from "lucide-react";
import type { AccountSummary, UsageEntry } from "../types";
import { formatResetTime } from "./AccountDetailHelpers";

function formatAge(secs: number): string {
  if (secs < 60) return `${Math.max(1, Math.round(secs))}s ago`;
  if (secs < 3600) return `${Math.round(secs / 60)}m ago`;
  return `${Math.round(secs / 3600)}h ago`;
}

export function SidebarAccountItem({
  account: a, active, usageEntry, cliBusy, reBusy,
  onSelect, onSwitchCli, onLogin, onRefreshUsage,
  onContextMenu, onBadgeContextMenu,
}: {
  account: AccountSummary;
  active: boolean;
  /**
   * Full usage entry (not just the 5h window). `null` when the account
   * has no credentials stored at all — that case is already surfaced
   * by the inline Log-in button to the right of the email, so this
   * component renders no usage row in that case.
   */
  usageEntry: UsageEntry | null;
  cliBusy: boolean;
  reBusy: boolean;
  onSelect: () => void;
  onSwitchCli: () => void;
  onLogin: () => void;
  /**
   * Retry the usage fetch for this account. Wired for the "Retry"
   * placeholder when status is "error" or "rate_limited". Optional:
   * when omitted, the Retry action is hidden.
   */
  onRefreshUsage?: () => void;
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
  const fiveHour = usageEntry?.usage?.five_hour ?? null;
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
        <UsageRow
          entry={usageEntry}
          fiveHourPct={fiveHourPct}
          resetsAt={fiveHour?.resets_at ?? null}
          onRefreshUsage={onRefreshUsage}
          onLogin={onLogin}
          onStopClick={stopClick}
          onStopKey={stopKeyActivation}
          onBadgeContextMenu={onBadgeContextMenu}
        />
      </div>
    </div>
  );
}

/**
 * Sidebar usage row. Renders one of:
 *
 * - the 5h bar when data is fresh,
 * - the bar + a muted "Nm ago" suffix when data is stale,
 * - a reason-keyed placeholder ("Token expired · Log in", "Rate-limited
 *   · retry in Ns", "Couldn't fetch · Retry") when data is unavailable.
 *
 * When the account has no usage entry at all (has_cli_credentials=false
 * upstream), the row renders nothing — the row-level Log-in button is
 * already the right affordance in that case.
 */
function UsageRow({
  entry,
  fiveHourPct,
  resetsAt,
  onRefreshUsage,
  onLogin,
  onStopClick,
  onStopKey,
  onBadgeContextMenu,
}: {
  entry: UsageEntry | null;
  fiveHourPct: number | null;
  resetsAt: string | null;
  onRefreshUsage?: () => void;
  onLogin: () => void;
  onStopClick: (e: React.MouseEvent, fn: () => void) => void;
  onStopKey: (e: React.KeyboardEvent) => void;
  onBadgeContextMenu?: (e: React.MouseEvent) => void;
}) {
  if (!entry) return null;

  const ctxHandler = onBadgeContextMenu
    ? (e: React.MouseEvent) => {
        e.stopPropagation();
        onBadgeContextMenu(e);
      }
    : undefined;

  if (entry.status === "ok" || entry.status === "stale") {
    const isStale = entry.status === "stale" && (entry.age_secs ?? 0) > 60;
    if (fiveHourPct !== null) {
      return (
        <div className="usage-bar-row" onContextMenu={ctxHandler}>
          <div className="usage-bar-container">
            <div className={`usage-bar-fill ${fiveHourPct >= 80 ? "high" : ""}`}
              style={{ width: `${Math.min(fiveHourPct, 100)}%` }} />
          </div>
          <span className={`usage-bar-label ${fiveHourPct >= 80 ? "high" : ""}`}>
            {Math.round(fiveHourPct)}%
            {resetsAt && <> · resets {formatResetTime(resetsAt)}</>}
            {isStale && entry.age_secs !== null && (
              <> <span className="usage-stale-chip" title="Showing cached data — Claudepot couldn't re-fetch just now">
                <Clock size={9} strokeWidth={2.5} /> {formatAge(entry.age_secs)}
              </span></>
            )}
          </span>
        </div>
      );
    }
    // Usage fetched successfully but the 5h window itself isn't
    // reported — e.g. free-tier account, or a new account that has not
    // yet run any requests in a 5h window. We explicitly must NOT
    // silently hide the row here; the whole point of this component is
    // to show state. The detail pane still renders the other windows.
    return (
      <div className="usage-placeholder-row" onContextMenu={ctxHandler}>
        <span className="usage-placeholder-msg muted">
          No activity in the last 5 hours
          {isStale && entry.age_secs !== null && (
            <> <span className="usage-stale-chip" title="Showing cached data">
              <Clock size={9} strokeWidth={2.5} /> {formatAge(entry.age_secs)}
            </span></>
          )}
        </span>
      </div>
    );
  }

  // Placeholder states. Identical slot height as the bar so rows don't
  // jump when data comes and goes.
  const status = entry.status;
  let message: React.ReactNode;
  let action: { label: string; onClick: () => void } | null = null;

  if (status === "expired") {
    message = (
      <>
        <AlertCircle size={11} strokeWidth={2.5} /> Token expired
      </>
    );
    action = { label: "Log in again", onClick: onLogin };
  } else if (status === "rate_limited") {
    const s = entry.retry_after_secs ?? 0;
    const hint = s > 60 ? `retry in ${Math.ceil(s / 60)}m` : `retry in ${s}s`;
    message = (
      <>
        <Clock size={11} strokeWidth={2.5} /> Rate-limited · {hint}
      </>
    );
    action = onRefreshUsage
      ? { label: "Retry", onClick: onRefreshUsage }
      : null;
  } else if (status === "error") {
    message = (
      <>
        <AlertCircle size={11} strokeWidth={2.5} /> Couldn't fetch usage
      </>
    );
    action = onRefreshUsage
      ? { label: "Retry", onClick: onRefreshUsage }
      : null;
  } else if (status === "no_credentials") {
    // Should not happen for rows that even render this component —
    // upstream filters these — but render a reasonable fallback.
    message = (
      <>
        <AlertCircle size={11} strokeWidth={2.5} /> No credentials
      </>
    );
    action = { label: "Log in", onClick: onLogin };
  } else {
    return null;
  }

  return (
    <div className="usage-placeholder-row" onContextMenu={ctxHandler}>
      <span className="usage-placeholder-msg muted">{message}</span>
      {action && (
        <button
          type="button"
          className="usage-placeholder-action link-btn"
          onClick={(e) => onStopClick(e, action!.onClick)}
          onKeyDown={onStopKey}
        >
          {action.label}
        </button>
      )}
    </div>
  );
}
