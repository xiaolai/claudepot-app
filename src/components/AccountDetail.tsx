import { Icon } from "./Icon";
import type { AccountSummary, UsageEntry } from "../types";
import { CollapsibleSection } from "./CollapsibleSection";
import { CopyButton } from "./CopyButton";
import { relativeTime, renderVerified, UsageRow } from "./AccountDetailHelpers";

function formatAge(secs: number): string {
  if (secs < 60) return `${Math.max(1, Math.round(secs))}s ago`;
  if (secs < 3600) return `${Math.round(secs / 60)}m ago`;
  return `${Math.round(secs / 3600)}h ago`;
}

export function AccountDetail({
  account: a,
  usageEntry,
  onRefreshUsage,
  onLogin,
}: {
  account: AccountSummary;
  usageEntry: UsageEntry | null;
  onRefreshUsage?: () => void;
  onLogin?: () => void;
}) {
  const hasAnomaly =
    a.drift ||
    a.verify_status === "rejected" ||
    a.verify_status === "drift" ||
    a.token_status === "expired" ||
    !a.credentials_healthy;

  // Render the Usage card in one of four modes:
  //   1. Data available (ok or stale)       — dl of windows + optional "as of Nm ago" chip
  //   2. Expired token                      — call-to-action: Log in again
  //   3. Rate-limited without cache         — countdown + Retry
  //   4. Error                              — error detail + Retry
  // Accounts with no credentials (has_cli_credentials=false) reach this
  // component with usageEntry === null — no card rendered, the login
  // affordance already exists in AccountActions.
  const usage = usageEntry?.usage ?? null;
  const showCard = usageEntry !== null;
  const isStale =
    usageEntry?.status === "stale" && (usageEntry.age_secs ?? 0) > 60;

  return (
    <>
      {showCard && (
        <CollapsibleSection
          title="Usage"
          titleSuffix={
            isStale && usageEntry!.age_secs !== null ? (
              <span
                className="usage-stale-chip"
                title="Showing cached data — Claudepot couldn't re-fetch just now"
              >
                <Icon name="clock" size={10} /> {formatAge(usageEntry!.age_secs!)}
              </span>
            ) : null
          }
          defaultOpen
        >
          {usage ? (
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
          ) : (
            <UsageUnavailable
              entry={usageEntry!}
              onRefresh={onRefreshUsage}
              onLogin={onLogin}
            />
          )}
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

/**
 * The Usage card's fallback when we know the account has credentials
 * but we can't render numbers. Message + primary action pair, one per
 * status. Never silent — the whole point of this redesign is the user
 * must know *why* usage isn't visible.
 */
function UsageUnavailable({
  entry,
  onRefresh,
  onLogin,
}: {
  entry: UsageEntry;
  onRefresh?: () => void;
  onLogin?: () => void;
}) {
  let body: React.ReactNode;
  let action: { label: string; onClick: () => void; icon?: React.ReactNode } | null =
    null;

  switch (entry.status) {
    case "expired":
      body = (
        <>
          <Icon name="alert-circle" size={14} />
          <span>
            Token expired. Log in again to refresh usage — the refresh token
            is rotated during login.
          </span>
        </>
      );
      if (onLogin) action = { label: "Log in again", onClick: onLogin };
      break;
    case "rate_limited": {
      const s = entry.retry_after_secs ?? 0;
      const hint = s > 60 ? `~${Math.ceil(s / 60)} minutes` : `${s} seconds`;
      body = (
        <>
          <Icon name="clock" size={14} />
          <span>
            Usage endpoint is rate-limiting Claudepot. Auto-retry in {hint}.
            Usage numbers reappear automatically once the cooldown clears.
          </span>
        </>
      );
      if (onRefresh) action = { label: "Retry now", onClick: onRefresh, icon: <Icon name="refresh" size={13} /> };
      break;
    }
    case "error":
      body = (
        <>
          <Icon name="alert-circle" size={14} />
          <span>
            Couldn't fetch usage.{" "}
            <code className="mono small">{entry.error_detail ?? "unknown"}</code>
          </span>
        </>
      );
      if (onRefresh) action = { label: "Retry", onClick: onRefresh, icon: <Icon name="refresh" size={13} /> };
      break;
    case "no_credentials":
      body = (
        <>
          <Icon name="alert-circle" size={14} />
          <span>
            No credentials stored for this account. Log in to populate usage.
          </span>
        </>
      );
      if (onLogin) action = { label: "Log in", onClick: onLogin };
      break;
    default:
      body = null;
  }

  return (
    <div className="usage-unavailable">
      <div className="usage-unavailable-body muted">{body}</div>
      {action && (
        <button
          type="button"
          className="usage-unavailable-action"
          onClick={action.onClick}
        >
          {action.icon} {action.label}
        </button>
      )}
    </div>
  );
}
