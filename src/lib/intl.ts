/**
 * Locale-bound `Intl` formatters — the one surface every user-facing
 * date, time, and number in `src/` formats through.
 *
 * `Intl.*Format(undefined, …)` and every bare `toLocale*String()`
 * resolve to the **OS** locale, not the UI language. On a Chinese Mac
 * that renders Chinese dates inside an English UI, and on an English
 * Mac it renders English dates inside a Chinese one. i18n-plan §8 rules
 * both out: dates and numbers follow the UI language. Routing through
 * this module is what keeps the locale argument from being forgotten
 * again — a fresh `new Intl.DateTimeFormat(undefined, …)` anywhere in
 * `src/` is a review finding.
 *
 * Formatter construction is not free and several call sites build one
 * per render, so instances are cached. **The cache key carries the
 * active locale.** Keying on the options alone would keep serving the
 * previous language's formatter after a switch — the whole defect this
 * module exists to fix, reintroduced one layer down.
 *
 * Currency is orthogonal to language. A cost figure is "what
 * pay-per-call would have cost", denominated in USD (or whatever code
 * the server reports for a billing balance) whatever the UI language
 * is; only the digit grouping and the symbol's placement localize.
 * Callers pass the currency code; this module never picks one.
 *
 * Not for machine-facing strings. Anything written to a file, a
 * clipboard payload another tool parses, a log line, or a filename
 * stays on a fixed format (ISO-8601) — those readers are not the user
 * and a locale switch must not move them.
 */
import { getActiveLocale } from "./i18n";

const dateTimeCache = new Map<string, Intl.DateTimeFormat>();
const numberCache = new Map<string, Intl.NumberFormat>();

/** Options serialize with sorted keys so two call sites that spell the
 *  same options in a different order share one instance. The locale
 *  leads the key — see the module note above. */
function cacheKey(locale: string, opts: object | undefined): string {
  if (!opts) return locale;
  const entries = Object.entries(opts).sort(([a], [b]) => (a < b ? -1 : 1));
  return `${locale} ${JSON.stringify(entries)}`;
}

/** A `Intl.DateTimeFormat` bound to the active UI locale. Reach for
 *  this only when you need the formatter object itself (e.g. to format
 *  many values); otherwise call {@link formatDateTime} and friends. */
export function dateTimeFormat(
  opts?: Intl.DateTimeFormatOptions,
): Intl.DateTimeFormat {
  const locale = getActiveLocale();
  const key = cacheKey(locale, opts);
  let fmt = dateTimeCache.get(key);
  if (!fmt) {
    fmt = new Intl.DateTimeFormat(locale, opts);
    dateTimeCache.set(key, fmt);
  }
  return fmt;
}

/** A `Intl.NumberFormat` bound to the active UI locale. Same guidance
 *  as {@link dateTimeFormat}: prefer {@link formatNumber} unless the
 *  formatter object is what you need (a currency formatter reused
 *  across several amounts in one render, say). */
export function numberFormat(
  opts?: Intl.NumberFormatOptions,
): Intl.NumberFormat {
  const locale = getActiveLocale();
  const key = cacheKey(locale, opts);
  let fmt = numberCache.get(key);
  if (!fmt) {
    fmt = new Intl.NumberFormat(locale, opts);
    numberCache.set(key, fmt);
  }
  return fmt;
}

// ECMA-402 spells out what `toLocaleString` / `toLocaleDateString` /
// `toLocaleTimeString` mean when given no options. Restating those
// three option sets here is what makes the helpers below drop-in
// replacements: `new Intl.DateTimeFormat(l, DEFAULT_*)` is
// byte-identical to the matching bare call, in en and zh-CN alike.
// `Intl.DateTimeFormat` with *no* options is date-only, so omitting
// them would silently drop the time.
const DEFAULT_DATE_TIME: Intl.DateTimeFormatOptions = {
  year: "numeric",
  month: "numeric",
  day: "numeric",
  hour: "numeric",
  minute: "numeric",
  second: "numeric",
};
const DEFAULT_DATE: Intl.DateTimeFormatOptions = {
  year: "numeric",
  month: "numeric",
  day: "numeric",
};
const DEFAULT_TIME: Intl.DateTimeFormatOptions = {
  hour: "numeric",
  minute: "numeric",
  second: "numeric",
};

/**
 * `Intl.DateTimeFormat.format` throws `RangeError` on an invalid date
 * where `toLocale*String` returns the string `"Invalid Date"`
 * (ECMA-402 mandates it). Transcript timestamps are arbitrary strings
 * from disk, so a malformed one must keep degrading to a visible
 * marker rather than taking the surface down.
 */
function formatChecked(
  value: Date | number,
  opts: Intl.DateTimeFormatOptions,
): string {
  const ms = typeof value === "number" ? value : value.getTime();
  // `isFinite`, not `!isNaN`: ±Infinity is outside the representable
  // date range, so `format` throws `RangeError` on it too. Guarding
  // only NaN left the degradation contract half-kept.
  if (!Number.isFinite(ms)) return "Invalid Date";
  return dateTimeFormat(opts).format(ms);
}

/** Date + time in the UI locale. Default options match a bare
 *  `Date.prototype.toLocaleString()`. */
export function formatDateTime(
  value: Date | number,
  opts: Intl.DateTimeFormatOptions = DEFAULT_DATE_TIME,
): string {
  return formatChecked(value, opts);
}

/** Date only, in the UI locale. Default options match a bare
 *  `Date.prototype.toLocaleDateString()`. */
export function formatDate(
  value: Date | number,
  opts: Intl.DateTimeFormatOptions = DEFAULT_DATE,
): string {
  return formatChecked(value, opts);
}

/** Time only, in the UI locale. Default options match a bare
 *  `Date.prototype.toLocaleTimeString()`. */
export function formatTime(
  value: Date | number,
  opts: Intl.DateTimeFormatOptions = DEFAULT_TIME,
): string {
  return formatChecked(value, opts);
}

/** Grouped number in the UI locale — the replacement for a bare
 *  `Number.prototype.toLocaleString()`. en and zh-CN happen to share
 *  `1,234` grouping, so this is invisible today; it stops being
 *  invisible the moment a locale that groups differently ships. */
export function formatNumber(
  value: number,
  opts?: Intl.NumberFormatOptions,
): string {
  return numberFormat(opts).format(value);
}
