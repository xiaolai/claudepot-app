// Compact human-readable relative-time formatter.
//
// Mirrors the convention used by the artifact-usage UI surfaces:
// "12s ago" / "5m ago" / "3h ago" / "2d ago". Tunable suffix so the
// same data can render with or without "ago" depending on context
// (table cells often skip the suffix to save horizontal space).
//
// Sub-second deltas are rounded to "now" / "0s" rather than spelled
// out — this isn't a profiler.

export interface FormatRelativeOptions {
  /** Append " ago" when true (default). Tables can pass false. */
  ago?: boolean;
}

export function formatRelative(
  ms: number,
  opts: FormatRelativeOptions = {},
): string {
  const ago = opts.ago ?? true;
  const diff = Date.now() - ms;
  if (diff < 0) return ago ? "just now" : "now";
  const sec = Math.floor(diff / 1000);
  const suffix = ago ? " ago" : "";
  if (sec < 60) return `${sec}s${suffix}`;
  const min = Math.floor(sec / 60);
  if (min < 60) return `${min}m${suffix}`;
  const hr = Math.floor(min / 60);
  if (hr < 48) return `${hr}h${suffix}`;
  return `${Math.floor(hr / 24)}d${suffix}`;
}
