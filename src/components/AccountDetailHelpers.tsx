import type * as React from "react";
import type { AccountSummary, UsageWindow } from "../types";

export function relativeTime(iso: string | null): string {
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

export function renderVerified(a: AccountSummary): React.ReactNode {
  const when = a.verified_at ? `· ${relativeTime(a.verified_at)}` : "";
  switch (a.verify_status) {
    case "ok":
      return <span className="verify-line ok">✓ {a.verified_email ?? "—"} {when}</span>;
    case "drift":
      return <span className="verify-line bad">DRIFT — blob authenticates as {a.verified_email ?? "?"} {when}</span>;
    case "rejected":
      return <span className="verify-line bad">server rejected token — re-login required {when}</span>;
    case "network_error":
      return <span className="verify-line muted">could not reach /profile {when} (last known: {a.verified_email ?? "—"})</span>;
    case "never":
    default:
      return <span className="muted">not yet verified</span>;
  }
}

function formatResetTime(iso: string): string {
  return new Date(iso).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
}

export function UsageRow({ label, window }: { label: string; window: UsageWindow | null }) {
  if (!window) return null;
  const pct = Math.round(window.utilization);
  return (
    <>
      <dt>{label}</dt>
      <dd>
        <span className={`usage-pct ${pct >= 80 ? "high" : ""}`}>{pct}%</span>
        {window.resets_at && (
          <> <span className="muted">resets {formatResetTime(window.resets_at)}</span></>
        )}
      </dd>
    </>
  );
}
