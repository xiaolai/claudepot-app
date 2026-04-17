import { useState, useCallback } from "react";
import {
  AlertTriangle,
  ChevronDown,
  ChevronRight,
  Lock,
  RefreshCw,
  Shield,
} from "lucide-react";
import { useStatusIssues, type StatusIssue } from "../hooks/useStatusIssues";
import type { AccountSummary, AppStatus, CcIdentity } from "../types";

const severityIcon = {
  error: <AlertTriangle size={14} strokeWidth={2} />,
  warning: <Shield size={14} strokeWidth={2} />,
  info: <RefreshCw size={14} strokeWidth={2} />,
};

function IssueAction({ action }: { action: StatusIssue["action"] }) {
  if (!action) return null;
  return (
    <button className="status-bar-action" onClick={action.onClick} title={action.label}>
      {action.label}
    </button>
  );
}

export function StatusBar({
  ccIdentity, status, syncError, keychainIssue, accounts, verifying, onUnlock,
}: {
  ccIdentity: CcIdentity | null; status: AppStatus | null;
  syncError: string | null; keychainIssue: string | null;
  accounts: AccountSummary[]; verifying: boolean; onUnlock: () => void;
}) {
  const [expanded, setExpanded] = useState(false);
  const issues = useStatusIssues({ ccIdentity, status, syncError, keychainIssue, accounts, onUnlock });
  const toggle = useCallback(() => setExpanded((p) => !p), []);

  if (issues.length === 0 && !verifying) return null;

  if (issues.length === 0 && verifying) {
    return (
      <div className="status-bar info" role="status" aria-live="polite">
        <RefreshCw size={14} className="status-bar-spin" />
        <span className="status-bar-text">Reconciling identities…</span>
      </div>
    );
  }

  const topSeverity = issues.some((i) => i.severity === "error") ? "error" : "warning";
  const verifyChip = verifying && (
    <span className="status-bar-verifying">
      <RefreshCw size={12} className="status-bar-spin" /> Reconciling…
    </span>
  );

  if (issues.length === 1) {
    const issue = issues[0];
    return (
      <div className={`status-bar ${issue.severity}`} role="alert" aria-live="assertive">
        {issue.severity === "error" ? <Lock size={14} strokeWidth={2} /> : severityIcon[issue.severity]}
        <span className="status-bar-text">
          {issue.label}
          {issue.detail && <span className="status-bar-detail"> — {issue.detail}</span>}
        </span>
        <IssueAction action={issue.action} />
        {verifyChip}
      </div>
    );
  }

  return (
    <div className={`status-bar ${topSeverity}`} role="alert" aria-live="assertive">
      <button className="status-bar-toggle" onClick={toggle}
        aria-expanded={expanded} title={expanded ? "Collapse issues" : "Expand issues"}>
        <AlertTriangle size={14} strokeWidth={2} />
        <span className="status-bar-text">{issues.length} issues</span>
        {expanded ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
      </button>
      {verifyChip}
      {expanded && (
        <ul className="status-bar-list">
          {issues.map((issue) => (
            <li key={issue.id} className={`status-bar-item ${issue.severity}`}>
              {severityIcon[issue.severity]}
              <div className="status-bar-item-content">
                <span>{issue.label}</span>
                {issue.detail && <span className="status-bar-detail">{issue.detail}</span>}
              </div>
              <IssueAction action={issue.action} />
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
