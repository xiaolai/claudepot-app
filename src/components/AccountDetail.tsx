import type * as React from "react";
import type { AccountSummary, AccountUsage, UsageWindow } from "../types";
import { CollapsibleSection } from "./CollapsibleSection";
import { CopyButton } from "./CopyButton";

function relativeTime(iso: string | null): string {
  if (!iso) return "\u2014";
  const diff = Date.now() - new Date(iso).getTime();
  const mins = Math.floor(diff / 60000);
  if (mins < 1) return "just now";
  if (mins < 60) return `${mins}m ago`;
  const hrs = Math.floor(mins / 60);
  if (hrs < 24) return `${hrs}h ago`;
  const days = Math.floor(hrs / 24);
  return `${days}d ago`;
}

function renderVerified(a: AccountSummary): React.ReactNode {
  const when = a.verified_at ? `· ${relativeTime(a.verified_at)}` : "";
  switch (a.verify_status) {
    case "ok":
      return (
        <span className="verify-line ok">
          ✓ {a.verified_email ?? "—"} {when}
        </span>
      );
    case "drift":
      return (
        <span className="verify-line bad">
          DRIFT — blob authenticates as {a.verified_email ?? "?"} {when}
        </span>
      );
    case "rejected":
      return (
        <span className="verify-line bad">
          server rejected token — re-login required {when}
        </span>
      );
    case "network_error":
      return (
        <span className="verify-line muted">
          could not reach /profile {when} (last known: {a.verified_email ?? "—"})
        </span>
      );
    case "never":
    default:
      return <span className="muted">not yet verified</span>;
  }
}

function formatResetTime(iso: string): string {
  const d = new Date(iso);
  return d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

function UsageRow({ label, window }: { label: string; window: UsageWindow | null }) {
  if (!window) return null;
  const pct = Math.round(window.utilization);
  return (
    <>
      <dt>{label}</dt>
      <dd>
        <span className={`usage-pct ${pct >= 80 ? "high" : ""}`}>{pct}%</span>
        {window.resets_at && (
          <>
            {" "}
            <span className="muted">resets {formatResetTime(window.resets_at)}</span>
          </>
        )}
      </dd>
    </>
  );
}

export function AccountDetail({
  account: a,
  usage,
}: {
  account: AccountSummary;
  usage: AccountUsage | null;
}) {
  const hasAnomaly =
    a.drift ||
    a.verify_status === "rejected" ||
    a.verify_status === "drift" ||
    a.token_status === "expired" ||
    !a.credentials_healthy;

  return (
    <>
      {/* Usage — always expanded (most-viewed data) */}
      {usage && (
        <CollapsibleSection title="Usage" defaultOpen>
          <dl className="detail-grid">
            <UsageRow label="5h window" window={usage.five_hour} />
            <UsageRow label="7d window" window={usage.seven_day} />
            <UsageRow label="7d Opus" window={usage.seven_day_opus} />
            <UsageRow label="7d Sonnet" window={usage.seven_day_sonnet} />
            {usage.extra_usage && (
              <>
                <dt>Extra</dt>
                <dd>
                  {usage.extra_usage.is_enabled
                    ? `$${(usage.extra_usage.used_credits ?? 0).toFixed(2)} / $${(usage.extra_usage.monthly_limit ?? 0).toFixed(2)}`
                    : "disabled"}
                </dd>
              </>
            )}
          </dl>
        </CollapsibleSection>
      )}

      {/* Identity — collapsed by default */}
      <CollapsibleSection title="Identity">
        <dl className="detail-grid">
          <dt>Email</dt>
          <dd className="selectable">{a.email} <CopyButton text={a.email} /></dd>
          <dt>UUID</dt>
          <dd className="mono selectable">{a.uuid} <CopyButton text={a.uuid} /></dd>
          <dt>Org</dt>
          <dd>{a.org_name ?? "\u2014"}</dd>
          <dt>Plan</dt>
          <dd>{a.subscription_type ?? "\u2014"}</dd>
          <dt>Last CLI switch</dt>
          <dd>{relativeTime(a.last_cli_switch)}</dd>
          <dt>Last Desktop switch</dt>
          <dd>{relativeTime(a.last_desktop_switch)}</dd>
        </dl>
      </CollapsibleSection>

      {/* Health — collapsed by default, auto-expands on anomaly */}
      <CollapsibleSection title="Health" forceOpen={hasAnomaly}>
        <dl className="detail-grid">
          <dt title="Local-clock check only — does not mean the server still accepts the token">Token</dt>
          <dd>
            {a.token_status}
            {a.token_status.startsWith("valid") && (
              <span className="muted verify-note"> (not past local expiry)</span>
            )}
          </dd>
          <dt>Verified</dt>
          <dd>{renderVerified(a)}</dd>
          <dt>Credentials</dt>
          <dd>{a.credentials_healthy ? "healthy" : "missing or corrupt"}</dd>
          <dt>Desktop profile</dt>
          <dd>{a.has_desktop_profile ? "present" : "none"}</dd>
        </dl>
      </CollapsibleSection>
    </>
  );
}
