import type { AccountSummary, AccountUsage } from "../types";
import { CollapsibleSection } from "./CollapsibleSection";
import { CopyButton } from "./CopyButton";
import { relativeTime, renderVerified, UsageRow } from "./AccountDetailHelpers";

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

      <CollapsibleSection title="Health" forceOpen={hasAnomaly}>
        <dl className="detail-grid">
          <dt title="Local-clock check only">Token</dt>
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
