import { useState, useCallback } from "react";
import {
  AlertTriangle,
  ChevronDown,
  ChevronRight,
  Lock,
  RefreshCw,
  Shield,
} from "lucide-react";
import type { AccountSummary, AppStatus, CcIdentity } from "../types";

export interface StatusIssue {
  id: string;
  severity: "error" | "warning" | "info";
  label: string;
  detail?: string;
  action?: { label: string; onClick: () => void };
}

function buildIssues(opts: {
  ccIdentity: CcIdentity | null;
  status: AppStatus | null;
  syncError: string | null;
  keychainIssue: string | null;
  accounts: AccountSummary[];
  verifying: boolean;
  onUnlock: () => void;
}): StatusIssue[] {
  const issues: StatusIssue[] = [];

  if (opts.keychainIssue) {
    issues.push({
      id: "keychain",
      severity: "error",
      label: "Keychain locked",
      detail: "Click Unlock to enter your macOS password.",
      action: { label: "Unlock", onClick: opts.onUnlock },
    });
  }

  const driftAccounts = opts.accounts.filter((a) => a.drift);
  if (driftAccounts.length > 0) {
    issues.push({
      id: "drift",
      severity: "error",
      label: "Account drift detected",
      detail: driftAccounts
        .map((a) => `${a.email} authenticates as ${a.verified_email}`)
        .join("; "),
    });
  }

  if (opts.syncError) {
    issues.push({
      id: "sync",
      severity: "warning",
      label: "Couldn't sync with Claude Code",
      detail: opts.syncError,
    });
  }

  if (
    opts.ccIdentity?.email &&
    opts.status?.cli_active_email &&
    opts.ccIdentity.email.toLowerCase() !==
      opts.status.cli_active_email.toLowerCase()
  ) {
    issues.push({
      id: "cc-drift",
      severity: "warning",
      label: `CC slot drift — CC authenticates as ${opts.ccIdentity.email}, Claudepot expects ${opts.status.cli_active_email}`,
    });
  }

  return issues;
}

const severityIcon = {
  error: <AlertTriangle size={14} strokeWidth={2} />,
  warning: <Shield size={14} strokeWidth={2} />,
  info: <RefreshCw size={14} strokeWidth={2} />,
};

export function StatusBar({
  ccIdentity,
  status,
  syncError,
  keychainIssue,
  accounts,
  verifying,
  onUnlock,
}: {
  ccIdentity: CcIdentity | null;
  status: AppStatus | null;
  syncError: string | null;
  keychainIssue: string | null;
  accounts: AccountSummary[];
  verifying: boolean;
  onUnlock: () => void;
}) {
  const [expanded, setExpanded] = useState(false);

  const issues = buildIssues({
    ccIdentity,
    status,
    syncError,
    keychainIssue,
    accounts,
    verifying,
    onUnlock,
  });

  const toggle = useCallback(() => setExpanded((p) => !p), []);

  // 0 issues + not verifying = no bar (clean state)
  if (issues.length === 0 && !verifying) return null;

  // Verifying only — small reconcile indicator
  if (issues.length === 0 && verifying) {
    return (
      <div className="status-bar info" role="status">
        <RefreshCw size={14} className="status-bar-spin" />
        <span className="status-bar-text">Reconciling identities…</span>
      </div>
    );
  }

  const topSeverity = issues.some((i) => i.severity === "error")
    ? "error"
    : "warning";

  // 1 issue = single line with inline action
  if (issues.length === 1) {
    const issue = issues[0];
    return (
      <div className={`status-bar ${issue.severity}`} role="alert">
        {issue.severity === "error" ? (
          <Lock size={14} strokeWidth={2} />
        ) : (
          severityIcon[issue.severity]
        )}
        <span className="status-bar-text">
          {issue.label}
          {issue.detail && (
            <span className="status-bar-detail"> — {issue.detail}</span>
          )}
        </span>
        {issue.action && (
          <button
            className="status-bar-action"
            onClick={issue.action.onClick}
          >
            {issue.action.label}
          </button>
        )}
        {verifying && (
          <span className="status-bar-verifying">
            <RefreshCw size={12} className="status-bar-spin" /> Reconciling…
          </span>
        )}
      </div>
    );
  }

  // Multiple issues = expandable
  return (
    <div className={`status-bar ${topSeverity}`} role="alert">
      <button className="status-bar-toggle" onClick={toggle}>
        <AlertTriangle size={14} strokeWidth={2} />
        <span className="status-bar-text">
          {issues.length} issues
        </span>
        {expanded ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
      </button>
      {verifying && (
        <span className="status-bar-verifying">
          <RefreshCw size={12} className="status-bar-spin" /> Reconciling…
        </span>
      )}
      {expanded && (
        <ul className="status-bar-list">
          {issues.map((issue) => (
            <li key={issue.id} className={`status-bar-item ${issue.severity}`}>
              {severityIcon[issue.severity]}
              <div className="status-bar-item-content">
                <span>{issue.label}</span>
                {issue.detail && (
                  <span className="status-bar-detail">{issue.detail}</span>
                )}
              </div>
              {issue.action && (
                <button
                  className="status-bar-action"
                  onClick={issue.action.onClick}
                >
                  {issue.action.label}
                </button>
              )}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
