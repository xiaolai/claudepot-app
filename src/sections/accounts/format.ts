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
 * Rate-limit reset time — calendar-app idiom, no weekday abbreviations.
 *
 *    past           → "due"
 *    < 60m          → "in 42m"              (pure relative)
 *    later today    → "today 17:30"          (wall clock + anchor)
 *    tomorrow       → "tomorrow 14:30"       (wall clock + anchor)
 *    ≥ 2 days out   → "Apr 23 14:30"         (locale-aware date + time)
 *
 * Inline form stays terse; `formatResetTooltip` carries the full
 * absolute `Apr 23, 2026, 14:30 GMT+08:00` on hover. Weekday labels
 * ("Mon 14:30") were dropped because they're English-only, force a
 * mental weekday→date step, and add no info the date itself doesn't.
 */
export function formatResetTime(iso: string | null | undefined): string {
  if (!iso) return "—";
  const d = new Date(iso);
  const now = new Date();
  const diffMs = d.getTime() - now.getTime();
  if (diffMs <= 0) return "due";
  const diffMins = Math.floor(diffMs / 60_000);
  if (diffMins < 60) return `in ${diffMins}m`;

  const time = d.toLocaleTimeString(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  });

  // Calendar-day diff in the user's local zone — *not* time-of-day
  // arithmetic. 23:50 → 00:10 the next morning is "tomorrow", not
  // "today" plus 20 minutes.
  const startOfToday = new Date(now);
  startOfToday.setHours(0, 0, 0, 0);
  const startOfResetDay = new Date(d);
  startOfResetDay.setHours(0, 0, 0, 0);
  const diffDays = Math.round(
    (startOfResetDay.getTime() - startOfToday.getTime()) / 86_400_000,
  );

  if (diffDays === 0) return `today ${time}`;
  if (diffDays === 1) return `tomorrow ${time}`;
  // For 2+ days out the exact time matters for planning — "resets
  // Apr 23 at 00:00 UTC" vs "Apr 23 14:30" are different decisions.
  // Space-separated so the form matches today/tomorrow idiom and
  // stays compact in the 96px reset column.
  const date = d.toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
  });
  return `${date} ${time}`;
}

/**
 * Long-form reset timestamp for a `title` tooltip. Combines an
 * absolute local datetime with the zone offset AND the relative
 * phrase from {@link formatResetTime} so the user sees both
 * "when exactly" and "how soon".
 *
 * Example: `"Resets Apr 20, 2026, 14:30 GMT+08:00 — in 3h 45m"`
 */
export function formatResetTooltip(iso: string | null | undefined): string {
  if (!iso) return "No reset scheduled";
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return "Reset time unknown";
  // `shortOffset` renders e.g. "GMT+08:00" (ES 2022, supported in all
  // Chromium/WebKit webviews Tauri ships against). Falls back to
  // timeZone short code ("EDT") on older runtimes.
  const absolute = new Intl.DateTimeFormat(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
    timeZoneName: "shortOffset",
  }).format(d);
  const diffMs = d.getTime() - Date.now();
  if (diffMs <= 0) return `Reset was ${absolute}`;
  const rel = humanDuration(diffMs);
  return `Resets ${absolute} — in ${rel}`;
}

/** Precise human phrasing for a forward-looking duration, e.g. "3h 45m". */
function humanDuration(ms: number): string {
  const totalMinutes = Math.max(1, Math.floor(ms / 60_000));
  const days = Math.floor(totalMinutes / 1440);
  const hours = Math.floor((totalMinutes % 1440) / 60);
  const mins = totalMinutes % 60;
  if (days > 0) return hours > 0 ? `${days}d ${hours}h` : `${days}d`;
  if (hours > 0) return mins > 0 ? `${hours}h ${mins}m` : `${hours}h`;
  return `${mins}m`;
}
