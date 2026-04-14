import type { AccountSummary } from "../types";
import { CopyButton } from "./CopyButton";

function relativeTime(iso: string | null): string {
  if (!iso) return "—";
  const diff = Date.now() - new Date(iso).getTime();
  const mins = Math.floor(diff / 60000);
  if (mins < 1) return "just now";
  if (mins < 60) return `${mins}m ago`;
  const hrs = Math.floor(mins / 60);
  if (hrs < 24) return `${hrs}h ago`;
  const days = Math.floor(hrs / 24);
  return `${days}d ago`;
}

export function AccountDetail({ account: a }: { account: AccountSummary }) {
  return (
    <div className="detail-section">
      <h3 className="detail-section-title">Details</h3>
      <dl className="detail-grid">
        <dt>Email</dt>
        <dd className="selectable">{a.email} <CopyButton text={a.email} /></dd>
        <dt>UUID</dt>
        <dd className="mono selectable">{a.uuid} <CopyButton text={a.uuid} /></dd>
        <dt>Org</dt>
        <dd>{a.org_name ?? "—"}</dd>
        <dt>Plan</dt>
        <dd>{a.subscription_type ?? "—"}</dd>
        <dt>Last CLI switch</dt>
        <dd>{relativeTime(a.last_cli_switch)}</dd>
        <dt>Last Desktop switch</dt>
        <dd>{relativeTime(a.last_desktop_switch)}</dd>
        <dt>Token</dt>
        <dd>{a.token_status}</dd>
        <dt>Credentials</dt>
        <dd>{a.credentials_healthy ? "healthy" : "missing or corrupt"}</dd>
        <dt>Desktop profile</dt>
        <dd>{a.has_desktop_profile ? "present" : "none"}</dd>
      </dl>
    </div>
  );
}
