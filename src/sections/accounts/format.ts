/**
 * "just now" / "3m ago" / "in 2h" / "5d ago"-style relative time.
 * Accepts an ISO 8601 timestamp; returns an em-dash for null/empty.
 */
export function relTime(iso: string | null | undefined): string {
  if (!iso) return "—";
  const diff = Date.now() - new Date(iso).getTime();
  const mins = Math.floor(Math.abs(diff) / 60_000);
  const future = diff < 0;
  if (mins < 1) return "just now";
  if (mins < 60) return future ? `in ${mins}m` : `${mins}m ago`;
  const hrs = Math.floor(mins / 60);
  if (hrs < 24) return future ? `in ${hrs}h` : `${hrs}h ago`;
  const days = Math.floor(hrs / 24);
  return future ? `in ${days}d` : `${days}d ago`;
}

/**
 * Rate-limit reset time formatter. "due" if already past,
 * "in 42m" within the next hour, clock time same day, weekday+time
 * within the week, month/day for further out.
 */
export function formatResetTime(iso: string | null | undefined): string {
  if (!iso) return "—";
  const d = new Date(iso);
  const now = new Date();
  const diffMs = d.getTime() - now.getTime();
  if (diffMs <= 0) return "due";
  const diffMins = Math.floor(diffMs / 60_000);
  if (diffMins < 60) return `in ${diffMins}m`;
  const t = d.toLocaleTimeString("en-US", {
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  });
  const sameDay = d.toDateString() === now.toDateString();
  if (sameDay) return t;
  const sot = new Date(now);
  sot.setHours(0, 0, 0, 0);
  const sor = new Date(d);
  sor.setHours(0, 0, 0, 0);
  const diffDays = Math.round((sor.getTime() - sot.getTime()) / 86_400_000);
  if (diffDays < 7) {
    const weekday = d.toLocaleDateString("en-US", { weekday: "short" });
    return `${weekday} ${t}`;
  }
  const md = d.toLocaleDateString("en-US", {
    month: "short",
    day: "numeric",
  });
  return `${md}, ${t}`;
}
