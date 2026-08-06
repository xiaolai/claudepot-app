// Compact human-readable relative-time formatter.
//
// Mirrors the convention used by the artifact-usage UI surfaces:
// "12s ago" / "5m ago" / "3h ago" / "2d ago". Tunable suffix so the
// same data can render with or without "ago" depending on context
// (table cells often skip the suffix to save horizontal space).
//
// Sub-second deltas are rounded to "now" / "0s" rather than spelled
// out — this isn't a profiler.

import { i18n } from "./i18n";

export interface FormatRelativeOptions {
  /** Append " ago" when true (default). Tables can pass false. */
  ago?: boolean;
}

/**
 * Two parallel key families rather than one key plus a suffix: not
 * every language builds "5m ago" by appending to "5m", so the
 * with-suffix form has to be translatable as a whole phrase.
 */
export function formatRelative(
  ms: number,
  opts: FormatRelativeOptions = {},
): string {
  const ago = opts.ago ?? true;
  const diff = Date.now() - ms;
  if (diff < 0) return i18n.t(ago ? "time.justNow" : "time.now");
  const sec = Math.floor(diff / 1000);
  if (sec < 60) return i18n.t(ago ? "time.secondsAgo" : "time.seconds", { n: sec });
  const min = Math.floor(sec / 60);
  if (min < 60) return i18n.t(ago ? "time.minutesAgo" : "time.minutes", { n: min });
  const hr = Math.floor(min / 60);
  if (hr < 48) return i18n.t(ago ? "time.hoursAgo" : "time.hours", { n: hr });
  return i18n.t(ago ? "time.daysAgo" : "time.days", { n: Math.floor(hr / 24) });
}
