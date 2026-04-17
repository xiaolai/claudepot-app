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

/**
 * Format a usage-window reset timestamp. Smart date awareness — the old
 * "HH:mm" form was ambiguous for 7-day windows (reset could be days
 * away). Granularity slides with distance:
 *
 *   < 1h   →  "in 43m"
 *   today  →  "14:30"
 *   < 7d   →  "Tue 14:30"
 *   ≥ 7d   →  "Apr 24, 14:30"
 *   past   →  "due" (reset already happened; CC will refresh on next call)
 */
export function formatResetTime(iso: string): string {
  const d = new Date(iso);
  const now = new Date();
  const diffMs = d.getTime() - now.getTime();
  if (diffMs <= 0) return "due";

  const diffMins = Math.floor(diffMs / 60_000);
  // Sub-hour: a countdown is more useful than wall-clock.
  if (diffMins < 60) return `in ${diffMins}m`;

  const timeStr = d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  const sameDay = d.toDateString() === now.toDateString();
  if (sameDay) return timeStr;

  // Days-of-year delta — more robust than `diffMs / 86_400_000` across DST.
  const startOfToday = new Date(now);
  startOfToday.setHours(0, 0, 0, 0);
  const startOfReset = new Date(d);
  startOfReset.setHours(0, 0, 0, 0);
  const diffDays = Math.round(
    (startOfReset.getTime() - startOfToday.getTime()) / 86_400_000,
  );

  if (diffDays < 7) {
    const weekday = d.toLocaleDateString([], { weekday: "short" });
    return `${weekday} ${timeStr}`;
  }
  return (
    d.toLocaleDateString([], { month: "short", day: "numeric" }) +
    `, ${timeStr}`
  );
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
