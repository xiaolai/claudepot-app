import { afterEach, describe, expect, it } from "vitest";
import { applyLocalePreference } from "./i18n";
import {
  dateTimeFormat,
  formatDate,
  formatDateTime,
  formatNumber,
  formatTime,
  numberFormat,
} from "./intl";

// A fixed instant, so en/zh-CN comparisons are of the same moment and
// the ambient timezone cancels out rather than needing to be pinned.
const INSTANT = new Date("2026-04-23T14:30:45.123Z");

// The runtime's *default* locale comes from the OS/ICU environment, not
// from `navigator.language`. The drop-in-equivalence assertions below
// only mean anything when that default is English; elsewhere they would
// be asserting a coincidence.
const RUNTIME_IS_ENGLISH = new Intl.DateTimeFormat()
  .resolvedOptions()
  .locale.startsWith("en");

afterEach(async () => {
  // The i18next instance is module-global and the formatter caches key
  // off it, so a leaked zh-CN would change another file's assertions.
  await applyLocalePreference(null);
});

describe("formatters follow the UI locale, not the OS", () => {
  it("renders a date-time differently in en and zh-CN", async () => {
    await applyLocalePreference("en");
    const en = formatDateTime(INSTANT);

    await applyLocalePreference("zh-CN");
    const zh = formatDateTime(INSTANT);

    // The defect this module exists to fix: before the conversion both
    // sides read the OS locale and this assertion could not fail.
    expect(zh).not.toBe(en);
    expect(en).toContain("2026");
    expect(zh).toContain("2026");
  });

  it("renders a date differently in en and zh-CN", async () => {
    await applyLocalePreference("en");
    const en = formatDate(INSTANT, { month: "short", day: "numeric" });

    await applyLocalePreference("zh-CN");
    const zh = formatDate(INSTANT, { month: "short", day: "numeric" });

    expect(zh).not.toBe(en);
  });

  it("renders a time differently in en and zh-CN", async () => {
    // en pins AM/PM at these options; zh-CN renders a 24-hour clock.
    await applyLocalePreference("en");
    const en = formatTime(INSTANT);

    await applyLocalePreference("zh-CN");
    const zh = formatTime(INSTANT);

    expect(zh).not.toBe(en);
  });

  it("switches back, so the change is not one-way", async () => {
    await applyLocalePreference("zh-CN");
    const zh = formatDateTime(INSTANT);

    await applyLocalePreference("en");
    expect(formatDateTime(INSTANT)).not.toBe(zh);
  });
});

describe("the formatter cache is keyed on the active locale", () => {
  const OPTS: Intl.DateTimeFormatOptions = {
    year: "numeric",
    month: "long",
    day: "numeric",
  };

  it("hands back a formatter for the NEW locale after a switch", async () => {
    await applyLocalePreference("en");
    const en = dateTimeFormat(OPTS);
    expect(en.resolvedOptions().locale).toBe("en");

    await applyLocalePreference("zh-CN");
    const zh = dateTimeFormat(OPTS);

    // Keying the cache on the options alone would return `en` here —
    // the original bug reintroduced one layer down.
    expect(zh.resolvedOptions().locale).toBe("zh-CN");
    expect(zh).not.toBe(en);
  });

  it("still caches within a locale", async () => {
    await applyLocalePreference("en");
    const first = dateTimeFormat(OPTS);

    await applyLocalePreference("zh-CN");
    dateTimeFormat(OPTS);

    // Returning the same instance proves the miss above was the locale
    // changing, not the cache being a no-op.
    await applyLocalePreference("en");
    expect(dateTimeFormat(OPTS)).toBe(first);
  });

  it("keys numbers on the locale too", async () => {
    const opts: Intl.NumberFormatOptions = { minimumFractionDigits: 2 };

    await applyLocalePreference("en");
    const en = numberFormat(opts);
    expect(en.resolvedOptions().locale).toBe("en");

    await applyLocalePreference("zh-CN");
    expect(numberFormat(opts).resolvedOptions().locale).toBe("zh-CN");
  });

  it("shares one instance across option keys spelled in any order", async () => {
    await applyLocalePreference("en");
    expect(dateTimeFormat({ day: "numeric", year: "numeric" })).toBe(
      dateTimeFormat({ year: "numeric", day: "numeric" }),
    );
  });
});

describe("currency is denominated, not translated", () => {
  it("keeps USD as USD in zh-CN", async () => {
    const opts: Intl.NumberFormatOptions = {
      style: "currency",
      currency: "USD",
    };

    await applyLocalePreference("en");
    expect(numberFormat(opts).resolvedOptions().currency).toBe("USD");

    await applyLocalePreference("zh-CN");
    // Only grouping and symbol placement localize; the code does not.
    expect(numberFormat(opts).resolvedOptions().currency).toBe("USD");
    expect(formatNumber(1234.5, opts)).toContain("1,234.50");
  });
});

describe("drop-in equivalence with the calls that were replaced", () => {
  it.skipIf(!RUNTIME_IS_ENGLISH)(
    "matches the bare toLocale* forms under en",
    async () => {
      await applyLocalePreference("en");
      // Every converted call site relied on this: the suite runs in
      // English, so the conversion must not move English output.
      expect(formatDateTime(INSTANT)).toBe(INSTANT.toLocaleString());
      expect(formatDate(INSTANT)).toBe(INSTANT.toLocaleDateString());
      expect(formatTime(INSTANT)).toBe(INSTANT.toLocaleTimeString());
      expect(formatNumber(1234567.891)).toBe((1234567.891).toLocaleString());
    },
  );

  it("degrades an invalid date instead of throwing", async () => {
    await applyLocalePreference("en");
    // `Intl.DateTimeFormat.format` throws RangeError where
    // `toLocaleString` returns this string, and transcript timestamps
    // are arbitrary strings from disk.
    expect(formatDateTime(new Date("not a date"))).toBe("Invalid Date");
    expect(formatDate(Number.NaN)).toBe("Invalid Date");
  });
});
