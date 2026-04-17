import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { formatResetTime } from "./AccountDetailHelpers";

/**
 * formatResetTime is date-aware to fix the 7-day-window ambiguity
 * (e.g. "resets 14:30" when the actual reset is three days out). Lock
 * down the granularity tiers here so a future locale / format change
 * doesn't silently regress back to time-only output.
 */
describe("formatResetTime", () => {
  const NOW = new Date("2026-04-17T10:00:00"); // Friday 10:00 local

  beforeEach(() => {
    vi.useFakeTimers();
    vi.setSystemTime(NOW);
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  // Build an ISO string for a local-time instant offset from NOW.
  function localIsoOffset(opts: { addHours?: number; addDays?: number }): string {
    const d = new Date(NOW);
    if (opts.addHours) d.setHours(d.getHours() + opts.addHours);
    if (opts.addDays) d.setDate(d.getDate() + opts.addDays);
    return d.toISOString();
  }

  it("returns 'due' when reset is in the past", () => {
    expect(formatResetTime(localIsoOffset({ addHours: -1 }))).toBe("due");
  });

  it("returns 'in Nm' when reset is less than an hour away", () => {
    const in43m = new Date(NOW);
    in43m.setMinutes(in43m.getMinutes() + 43);
    expect(formatResetTime(in43m.toISOString())).toBe("in 43m");
  });

  it("returns time-only when reset is later today", () => {
    // 14:30 same day — the common 5h window case.
    const sameDay = new Date(NOW);
    sameDay.setHours(14, 30, 0, 0);
    const formatted = formatResetTime(sameDay.toISOString());
    // Locale-dependent exact form; assert the digits rather than a
    // specific separator to stay robust across "14:30" / "2:30 PM".
    expect(formatted).toMatch(/14|2:30|02:30/);
    // No weekday / month name should be present on a same-day reset.
    expect(formatted).not.toMatch(/Mon|Tue|Wed|Thu|Fri|Sat|Sun/);
  });

  it("includes the weekday when reset is within 7 days", () => {
    // +3 days → Monday (Apr 20, 2026).
    const formatted = formatResetTime(localIsoOffset({ addDays: 3 }));
    // Short weekday: "Mon"
    expect(formatted).toMatch(/Mon/);
  });

  it("includes month+day when reset is 7 or more days away", () => {
    const formatted = formatResetTime(localIsoOffset({ addDays: 9 }));
    // Expect month short-name (Apr or May depending on locale) to
    // appear — assert on "2026" would be fragile since we chose
    // {month: "short", day: "numeric"}.
    expect(formatted).toMatch(/Apr|May/);
  });
});
